//! VP6 embedded-video decoding — ported from Ruffle's `ruffle_video_software`
//! `decoder/vp6.rs` (Ruffle is MIT/Apache-2.0 licensed; credit to the
//! Ruffle project: https://github.com/ruffle-rs/ruffle).
//!
//! `DefineVideoStream` characters (bonus boxes, ores, explosions) carry a
//! sequence of `VideoFrame` payloads. VP6 is inter-coded, so frames are
//! decoded strictly in frame-number order and `last_frame` reuse frames
//! (empty payloads) share the previously decoded buffer.
//!
//! Output frames are cropped to the stream's declared bounds and converted
//! from YUV(A)420p to premultiplied RGBA — the same format tiny-skia
//! `Pixmap`s store, so a `Pixmap` can be built directly from the bytes.

use h263_rs_yuv::bt601::yuv420_to_rgba;
use nihav_codec_support::codecs::NABufferType::Video;
use nihav_codec_support::codecs::{NABufferRef, NAVideoBuffer, NAVideoInfo, YUV420_FORMAT};
use nihav_core::codecs::NADecoderSupport;
use nihav_duck::codecs::vp6::{VP56Decoder, VP56Parser, VP6BR};
use nihav_duck::codecs::vpcommon::{BoolCoder, VP_YUVA420_FORMAT};
use swf::VideoCodec;
use tiny_skia::Pixmap;

#[derive(Debug)]
pub enum Vp6Error {
    /// DecoderError doesn't impl Error... so this is manual.
    DecoderError(nihav_core::codecs::DecoderError),
    UnexpectedSkipFrame,
    InvalidBufferType,
}

impl std::fmt::Display for Vp6Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DecoderError(e) => write!(f, "decoder error: {e:?}"),
            Self::UnexpectedSkipFrame => write!(f, "unexpected skip frame"),
            Self::InvalidBufferType => write!(f, "invalid buffer type"),
        }
    }
}

impl std::error::Error for Vp6Error {}

impl From<nihav_core::codecs::DecoderError> for Vp6Error {
    fn from(error: nihav_core::codecs::DecoderError) -> Self {
        Vp6Error::DecoderError(error)
    }
}

/// VP6 video decoder.
struct Vp6Decoder {
    with_alpha: bool,
    bounds: (u16, u16),
    decoder: VP56Decoder,
    support: NADecoderSupport,
    bitreader: VP6BR,
    init_called: bool,
    last_frame: Option<NABufferRef<NAVideoBuffer<u8>>>,
    scratch: Vec<u8>,
}

impl Vp6Decoder {
    fn new(with_alpha: bool, bounds: (u16, u16)) -> Self {
        // Unfortunately, `init()` cannot be called on the decoder
        // just yet, because `bounds` is only the declared size of
        // the video, to which it will be cropped.
        // This can be (rarely) even much smaller than the actual
        // encoded size of the frames.
        // `VP56Decoder::init()` needs the full encoded frame size,
        // so it can allocate its internal buffers accordingly.
        // The encoded frame size will be parsed from the header of
        // the first encoded frame passed to `Self::decode_frame()`.
        Self {
            with_alpha,
            bounds,
            decoder: VP56Decoder::new(6, with_alpha, true),
            support: NADecoderSupport::new(),
            bitreader: VP6BR::new(),
            init_called: false,
            last_frame: None,
            scratch: Vec::new(),
        }
    }

    /// Decode one encoded frame into the cropped YUV(A)420p `scratch`
    /// buffer and return `(width, height)` of the cropped picture.
    fn decode_frame(&mut self, data: &[u8]) -> Result<(usize, usize), Vp6Error> {
        // If this is the first frame, the decoder needs to be initialized.
        if !self.init_called {
            let mut bool_coder = BoolCoder::new(if self.with_alpha {
                // The 24 bits alpha offset needs to be skipped first in this case
                data.get(3..).ok_or(Vp6Error::DecoderError(
                    nihav_core::codecs::DecoderError::InvalidData,
                ))?
            } else {
                data
            })
            .map_err(Vp6Error::DecoderError)?;

            let header = self
                .bitreader
                .parse_header(&mut bool_coder)
                .map_err(Vp6Error::DecoderError)?;

            // NOTE: It seems that the disp_w and disp_h fields in the header
            // are just ignored by Flash Player (and by VLC too), whatever the
            // VP6 spec has to say about ScalingMode functionality...
            let video_info = NAVideoInfo::new(
                header.mb_w as usize * 16,
                header.mb_h as usize * 16,
                true,
                if self.with_alpha {
                    VP_YUVA420_FORMAT
                } else {
                    YUV420_FORMAT
                },
            );

            self.decoder
                .init(&mut self.support, video_info)
                .map_err(Vp6Error::DecoderError)?;

            self.init_called = true;
        }

        let frame = if data.is_empty() || (self.with_alpha && data.len() <= 3) {
            // This frame is empty, so it's a "skip frame"; reusing the last frame, if there is one.
            self.last_frame
                .as_ref()
                .ok_or(Vp6Error::UnexpectedSkipFrame)?
        } else {
            // Actually decoding the frame and extracting the buffer it is stored in.
            let decoded = self
                .decoder
                .decode_frame(&mut self.support, data, &mut self.bitreader)
                .map_err(Vp6Error::DecoderError)?;

            let frame = match decoded {
                (Video(buffer), _) => Ok(buffer),
                _ => Err(Vp6Error::InvalidBufferType),
            }?;

            self.last_frame.insert(frame)
        };

        let yuv = frame.get_data();

        let (mut width, mut height) = frame.get_dimensions(0);
        let (chroma_width, chroma_height) = frame.get_dimensions(1);
        debug_assert_eq!(chroma_width, width.div_ceil(2));
        debug_assert_eq!(chroma_height, height.div_ceil(2));

        // We assume that there is no padding between rows
        debug_assert!(frame.get_stride(0) == frame.get_dimensions(0).0);
        debug_assert!(frame.get_stride(1) == frame.get_dimensions(1).0);
        debug_assert!(frame.get_stride(2) == frame.get_dimensions(2).0);

        // Where each plane starts in the buffer
        let offsets = (
            frame.get_offset(0),
            frame.get_offset(1),
            frame.get_offset(2),
        );

        let y = &yuv[offsets.0..offsets.0 + width * height];
        let u = &yuv[offsets.1..offsets.1 + chroma_width * chroma_height];
        let v = &yuv[offsets.2..offsets.2 + chroma_width * chroma_height];

        // Cropping the encoded frame (containing whole macroblocks) to the
        // size requested by the bounds attribute.
        let &bounds = &self.bounds;

        if width < bounds.0 as usize || height < bounds.1 as usize {
            log::warn!(
                "A VP6 video frame is smaller than the bounds of the stream it belongs in. This is not supported."
            );
            // Flash Player just produces a black image in this case!
        }

        self.scratch.clear();

        crop(&mut self.scratch, y, width, bounds);
        crop(
            &mut self.scratch,
            u,
            chroma_width,
            (bounds.0.div_ceil(2), bounds.1.div_ceil(2)),
        );
        crop(
            &mut self.scratch,
            v,
            chroma_width,
            (bounds.0.div_ceil(2), bounds.1.div_ceil(2)),
        );

        width = bounds.0 as usize;
        height = bounds.1 as usize;

        // Adding in the alpha component, if present.
        if self.with_alpha {
            // Apparently it's possible for the alpha channel to be coded in a different size than the Y channel.
            let (alpha_width, alpha_height) = frame.get_dimensions(3);
            debug_assert!(frame.get_stride(3) == frame.get_dimensions(3).0);

            let alpha_offset = frame.get_offset(3);
            let alpha = &yuv[alpha_offset..alpha_offset + alpha_width * alpha_height];
            crop(&mut self.scratch, alpha, alpha_width, bounds);
        }

        Ok((width, height))
    }
}

/// Crop `data` to the requested size, and append the result to the `out` buffer.
fn crop(out: &mut Vec<u8>, data: &[u8], width: usize, to_size: (u16, u16)) {
    debug_assert!(data.len() % width == 0);
    let height = data.len() / width;

    let new_width = usize::min(width, to_size.0 as usize);
    let new_height = usize::min(height, to_size.1 as usize);

    if width > to_size.0 as usize {
        // Removing the unwanted pixels on the right edge (most commonly: unused pieces
        // of macroblocks) by squishing all the rows tightly next to each other.
        // Bitmap at the moment does not allow these gaps, so we need to remove them.
        out.reserve(new_width * new_height);

        // No need to move rows on the bottom that are being cropped entirely
        for row in 0..new_height {
            let row_data = &data[row * width..(row * width + new_width)];
            out.extend_from_slice(row_data);
        }
    } else {
        // Cropping unwanted rows on the bottom
        // No horizontal squish needed, everything can be copied in one go
        out.extend_from_slice(&data[..width * new_height]);
    }
}

/// Convert the cropped YUV(A)420p `scratch` to a premultiplied RGBA
/// `Pixmap` (`r,g,b` are clamped to `a` so the result is valid
/// premultiplied data — matching tiny-skia's storage).
fn scratch_to_pixmap(
    scratch: &[u8],
    width: usize,
    height: usize,
    with_alpha: bool,
) -> Option<Pixmap> {
    let luma_len = width * height;
    let chroma_len = width.div_ceil(2) * height.div_ceil(2);
    if scratch.len() < luma_len + 2 * chroma_len {
        return None;
    }

    let y = &scratch[0..luma_len];
    let u = &scratch[luma_len..luma_len + chroma_len];
    let v = &scratch[luma_len + chroma_len..luma_len + 2 * chroma_len];

    let rgba = yuv420_to_rgba(y, u, v, width);

    let data: Vec<u8> = if with_alpha {
        let a = &scratch[luma_len + 2 * chroma_len..2 * luma_len + 2 * chroma_len];
        if a.len() < luma_len {
            return None;
        }
        // RGB components need to be clamped to alpha to avoid invalid premultiplied colors
        rgba.as_chunks::<4>()
            .0
            .iter()
            .zip(a)
            .flat_map(|(rgba, a)| [rgba[0].min(*a), rgba[1].min(*a), rgba[2].min(*a), *a])
            .collect()
    } else {
        // yuv420_to_rgba already emits alpha 255.
        rgba
    };

    let size = tiny_skia::IntSize::from_wh(width as u32, height as u32)?;
    Pixmap::from_vec(data, size)
}

/// Decode a `DefineVideoStream`'s `VideoFrame` payloads into RGBA pixmaps,
/// indexed by frame number (`Vec[i]` = frame `i`, `None` when that frame
/// failed to decode or was never sent).
///
/// `frames` is `(frame_num, encoded_data)` pairs, decoded in frame-number
/// order regardless of input order — VP6 is inter-coded.
pub fn decode_stream(
    codec: VideoCodec,
    width: u16,
    height: u16,
    frames: &[(u16, &[u8])],
) -> Vec<Option<Pixmap>> {
    let max_frame = frames.iter().map(|(n, _)| *n).max().unwrap_or(0);
    let mut out = vec![None; max_frame as usize + 1];

    let with_alpha = match codec {
        VideoCodec::Vp6 => false,
        VideoCodec::Vp6WithAlpha => true,
        _ => {
            log::warn!("swf_extract: unsupported embedded video codec {codec:?}");
            return out;
        }
    };

    let mut decoder = Vp6Decoder::new(with_alpha, (width, height));

    let mut ordered: Vec<&(u16, &[u8])> = frames.iter().collect();
    ordered.sort_by_key(|(n, _)| *n);

    for (frame_num, data) in ordered {
        let pm = match decoder.decode_frame(data) {
            Ok((w, h)) => {
                let pm = scratch_to_pixmap(&decoder.scratch, w, h, with_alpha);
                if pm.is_none() {
                    log::warn!("swf_extract: failed to convert video frame {frame_num}");
                }
                pm
            }
            Err(e) => {
                log::warn!("swf_extract: video frame {frame_num} decode failed: {e}");
                None
            }
        };
        if let Some(slot) = out.get_mut(*frame_num as usize) {
            *slot = pm;
        }
    }

    out
}
