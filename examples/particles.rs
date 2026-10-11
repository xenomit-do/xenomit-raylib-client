//! `particles <res-dir> <out.png> <group>...` — render particle groups from loose
//! files (`<id>.zip`, `<id>.atf`, `<id>.awd`) with the real particle renderer
//! and camera, to check effects without a live session.
use raylib::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use xenomit_client::assets::particles::{ParticleGroup, Shape};
use xenomit_client::assets::{atf, awd};
use xenomit_client::render::particles3d::{self, ParticleResources, Request};
use xenomit_client::render::scene3d;

struct Loose {
    groups: HashMap<String, Arc<ParticleGroup>>,
    textures: HashMap<String, Texture2D>,
    models: HashMap<(String, String), Model>,
}

impl ParticleResources for Loose {
    fn group(&self, id: &str) -> Option<Arc<ParticleGroup>> {
        self.groups.get(id).cloned()
    }

    fn texture_id(&self, id: &str) -> Option<u32> {
        self.textures.get(id).map(|t| t.id)
    }

    fn model(&self, geometry: &str, texture: &str) -> Option<&Model> {
        self.models.get(&(geometry.to_owned(), texture.to_owned()))
    }
}

fn upload(rl: &mut RaylibHandle, thread: &RaylibThread, image: &image::RgbaImage) -> Texture2D {
    let (w, h) = image.dimensions();
    unsafe {
        let data = raylib::ffi::MemAlloc(image.as_raw().len() as u32);
        std::ptr::copy_nonoverlapping(
            image.as_raw().as_ptr(),
            data as *mut u8,
            image.as_raw().len(),
        );
        let img = Image::from_raw(raylib::ffi::Image {
            data,
            width: w as i32,
            height: h as i32,
            mipmaps: 1,
            format: raylib::ffi::PixelFormat::PIXELFORMAT_UNCOMPRESSED_R8G8B8A8 as i32,
        });
        rl.load_texture_from_image(thread, &img).expect("texture")
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("res dir");
    let out = args.next().expect("out.png");
    let ids: Vec<String> = args.collect();
    let (mut rl, thread) = raylib::init().size(1280, 720).title("particles").build();
    rl.set_target_fps(60);

    let mut res = Loose {
        groups: HashMap::new(),
        textures: HashMap::new(),
        models: HashMap::new(),
    };
    for id in &ids {
        let group =
            ParticleGroup::from_bytes(&std::fs::read(format!("{dir}/{id}.zip")).unwrap()).unwrap();
        for anim in &group.anims {
            if let Some(t) = &anim.texture {
                if !res.textures.contains_key(t) {
                    let image =
                        atf::decode(&std::fs::read(format!("{dir}/{t}.atf")).unwrap()).unwrap();
                    res.textures
                        .insert(t.clone(), upload(&mut rl, &thread, &image));
                }
            }
            if let Shape::External(g) = &anim.shape {
                let key = (g.clone(), anim.texture.clone().unwrap_or_default());
                if !res.models.contains_key(&key) {
                    let mesh =
                        awd::parse_aw2(&std::fs::read(format!("{dir}/{g}.awd")).unwrap()).unwrap();
                    let obj = std::env::temp_dir().join(format!("particle-{g}.obj"));
                    std::fs::write(&obj, mesh.to_obj()).unwrap();
                    let mut model = rl.load_model(&thread, &obj.to_string_lossy()).unwrap();
                    if let Some(t) = res.textures.get(&key.1) {
                        for m in model.materials_mut() {
                            m.set_material_texture(MaterialMapIndex::MATERIAL_MAP_ALBEDO, t);
                        }
                    }
                    res.models.insert(key, model);
                }
            }
        }
        res.groups.insert(id.clone(), Arc::new(group));
    }

    for frame in 0..150 {
        let camera = scene3d::orbit_camera((5000.0, 5000.0), 2.0, 720.0, true);
        let requests: Vec<Request> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| Request {
                key: format!("view:{i}"),
                resource: id.clone(),
                pos: (4800.0 + 200.0 * i as f32, 0.0, -5000.0),
                rot: (0.0, 0.0, 0.0),
                scale: (1.0, 1.0, 1.0),
            })
            .collect();
        let mut d = rl.begin_drawing(&thread);
        d.clear_background(Color::new(8, 10, 22, 255));
        {
            let mut m3 = d.begin_mode3D(camera);
            for i in -8..=8 {
                let t = i as f32 * 100.0;
                m3.draw_line_3D(
                    scene3d::rl_point(4200.0, 0.0, -(5000.0 + t)),
                    scene3d::rl_point(5800.0, 0.0, -(5000.0 + t)),
                    Color::new(40, 40, 70, 255),
                );
                m3.draw_line_3D(
                    scene3d::rl_point(5000.0 + t, 0.0, -4200.0),
                    scene3d::rl_point(5000.0 + t, 0.0, -5800.0),
                    Color::new(40, 40, 70, 255),
                );
            }
        }
        particles3d::render(&mut d, camera, &requests, &res);
        if frame == 149 {
            unsafe {
                raylib::ffi::rlDrawRenderBatchActive();
                let img = raylib::ffi::LoadImageFromScreen();
                let c = std::ffi::CString::new(out.clone()).unwrap();
                raylib::ffi::ExportImage(img, c.as_ptr());
                raylib::ffi::UnloadImage(img);
            }
        }
    }
    println!("wrote {out}");
}
