#!/usr/bin/env python3
"""Build a searchable asset catalogue; optionally fetch every CDN-indexed file."""
from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import re
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
CACHE = ROOT / ".xenomit-assets"
OUT = ROOT / "asset-catalog"
PREVIEWS = OUT / "previews"
NATIVE_PREVIEWS = CACHE / "catalog-previews"
SWF_RASTER_CACHE = CACHE / "catalog-swf"
SWF_INPUTS = CACHE / "catalog-swf-inputs.json"
CELL_W, CELL_H = 180, 142
ATLAS_COLS, ATLAS_ROWS = 5, 12
ATLAS_PAGE_SIZE = ATLAS_COLS * ATLAS_ROWS


def asset_id(prefix: str, value: str) -> str:
    return f"{prefix}-{hashlib.sha1(value.encode('utf-8')).hexdigest()[:12]}"


def variants(folder: Path) -> dict[str, list[Path]]:
    groups: dict[str, list[Path]] = defaultdict(list)
    for path in folder.iterdir():
        if path.is_file():
            canonical = re.sub(r"__(?:h[0-9a-fA-F]*|v[^/]+)$", "", path.name)
            groups[canonical].append(path)
    return groups


def best_variant(paths: list[Path]) -> Path:
    return max(paths, key=lambda p: p.stat().st_mtime_ns)


def preview_is_fresh(target: Path, sources: list[Path]) -> bool:
    if not target.is_file():
        return False
    newest_source = max((p.stat().st_mtime_ns for p in sources if p.is_file()), default=0)
    if target.stat().st_mtime_ns < newest_source:
        return False
    try:
        with Image.open(target) as image:
            image.verify()
        return True
    except Exception:
        target.unlink(missing_ok=True)
        return False


def safe_file_id(value: str) -> str:
    return re.sub(r"[\\/:*?\"<>|]", "_", value)


def load_cdn_index():
    metadata_path = CACHE / "resource-catalog.json"
    xml_path = CACHE / "resources.xml"
    if not metadata_path.is_file() or not xml_path.is_file():
        return None, []
    try:
        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        root = ET.parse(xml_path).getroot()
    except (OSError, json.JSONDecodeError, ET.ParseError) as exc:
        print(f"Could not read saved CDN resource index: {exc}")
        return None, []
    locations = {}
    for node in root.iter():
        if node.tag.split("}")[-1] == "location" and node.get("id") is not None:
            locations[node.get("id")] = node.get("path", "")
    entries = {}
    for node in root.iter():
        if node.tag.split("}")[-1] != "file" or node.get("id") is None:
            continue
        loc = node.get("location", "")
        entries[node.get("id", "")] = {
            "id": node.get("id", ""), "location": loc,
            "path": locations.get(loc, ""), "name": node.get("name", ""),
            "type": node.get("type", ""), "version": node.get("version", ""),
            "hash": node.get("hash", ""), "hash_present": "hash" in node.attrib,
        }
    return metadata, list(entries.values())


def resource_target(entry: dict) -> Path:
    base = safe_file_id(entry["id"])
    if entry["hash_present"]:
        suffix = "__h" + safe_file_id(entry["hash"])
    elif entry["version"]:
        suffix = "__v" + safe_file_id(entry["version"])
    else:
        suffix = ""
    return CACHE / "files" / (base + suffix)


def resource_url(metadata: dict, entry: dict) -> str:
    rel = entry["path"] + entry["name"] + ("." + entry["type"] if entry["type"] else "")
    if rel.startswith(("/", "\\")) or any(p == ".." for p in re.split(r"[/\\]", rel)):
        raise ValueError(f"unsafe CDN path for {entry['id']!r}")
    base = metadata["resource_base"].rstrip("/") + "/"
    url = base + urllib.parse.quote(rel.lstrip("/"), safe="/!$&'()*+,;=:@-._~")
    if entry["hash_present"]:
        url += "?__cv=" + urllib.parse.quote(entry["hash"], safe="")
    return url


def download_one(metadata: dict, entry: dict):
    target = resource_target(entry)
    if target.is_file():
        return "cached", None
    try:
        url = resource_url(metadata, entry)
    except ValueError as exc:
        return "failed", str(exc)
    target.parent.mkdir(parents=True, exist_ok=True)
    temp = target.with_name(target.name + ".part")
    for attempt in range(4):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": "XenomitAssetCatalog/1.0"})
            with urllib.request.urlopen(request, timeout=60) as response, temp.open("wb") as out:
                while chunk := response.read(1024 * 1024):
                    out.write(chunk)
            if not temp.stat().st_size:
                temp.replace(target)
                return "empty", "HTTP 200 returned an empty body; saved to avoid retrying it every run"
            temp.replace(target)
            return "downloaded", None
        except urllib.error.HTTPError as exc:
            if exc.code not in (408, 425, 429, 500, 502, 503, 504) or attempt == 3:
                temp.unlink(missing_ok=True)
                return "failed", f"HTTP {exc.code}"
        except Exception as exc:
            if attempt == 3:
                temp.unlink(missing_ok=True)
                return "failed", str(exc)
        time.sleep(min(8, 0.75 * (2 ** attempt)))
    return "failed", "retry limit reached"


def download_all_resources(metadata: dict, entries: list[dict]):
    if not metadata or not metadata.get("resource_base"):
        raise SystemExit(
            "No saved CDN index yet. Launch the updated client and let it reach the game once; "
            "it will save resources.xml and its public CDN base under .xenomit-assets."
        )
    counts = {"downloaded": 0, "cached": 0, "empty": 0, "failed": 0}
    issues = []
    print(f"Fetching {len(entries)} indexed files from {metadata['resource_base']} (8 concurrent requests).", flush=True)
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        futures = {pool.submit(download_one, metadata, entry): entry for entry in entries}
        for i, future in enumerate(concurrent.futures.as_completed(futures), 1):
            status, error = future.result()
            counts[status] += 1
            if error and len(issues) < 40:
                issues.append((status, futures[future]["id"], error))
            if i % 50 == 0 or i == len(entries):
                print(f"  {i}/{len(entries)} · {counts['downloaded']} downloaded · {counts['cached']} cached · {counts['empty']} empty · {counts['failed']} failed", flush=True)
    for status, file_id, error in issues:
        print(f"  {status} {file_id}: {error}")
    print("CDN fetch complete: " + ", ".join(f"{k}={v}" for k, v in counts.items()), flush=True)


def indexed_resource_entries(metadata, entries, path_by_id, raw_files):
    rows = []
    raw_by_name = {row["name"]: row for row in raw_files}
    for entry in entries:
        target = resource_target(entry)
        canonical = CACHE / "files" / safe_file_id(entry["id"])
        cached_path = target if target.is_file() else path_by_id.get(safe_file_id(entry["id"]))
        if cached_path is None and canonical.is_file():
            cached_path = canonical
        row = dict(entry)
        row["cached"] = cached_path is not None
        row["cache_path"] = cached_path.relative_to(ROOT).as_posix() if cached_path else None
        row["cache_bytes"] = cached_path.stat().st_size if cached_path else None
        row["kind"] = entry["type"].upper() or "unknown"
        raw = raw_by_name.get(safe_file_id(entry["id"]))
        row["preview"] = raw.get("preview") if raw else None
        row["preview_reason"] = raw.get("preview_reason") if raw else (
            "File is not cached locally; download it to generate a preview."
            if not cached_path else missing_preview_reason(row["kind"])
        )
        try:
            row["url"] = resource_url(metadata, entry) if metadata and metadata.get("resource_base") else None
        except (ValueError, KeyError):
            row["url"] = None
        rows.append(row)
    return rows


def version_number(path: Path) -> int:
    match = re.match(r"v(\d+)-", path.parent.name)
    return int(match.group(1)) if match else 0


def font(size: int = 12):
    for name in ("segoeui.ttf", "arial.ttf", "DejaVuSans.ttf"):
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            pass
    return ImageFont.load_default()


def checker(draw, box, size=12):
    x0, y0, x1, y1 = box
    colors = ((35, 43, 55, 255), (49, 58, 71, 255))
    for y in range(y0, y1, size):
        for x in range(x0, x1, size):
            draw.rectangle((x, y, min(x + size, x1), min(y + size, y1)),
                           fill=colors[((x-x0)//size + (y-y0)//size) % 2])


def save_clip_preview(frames: list[Path], durations: list[int], target: Path) -> bool:
    if not frames:
        return False
    if preview_is_fresh(target, frames):
        return True
    n = min(6, len(frames))
    indexes = [0] if n == 1 else sorted({round(i * (len(frames)-1)/(n-1)) for i in range(n)})
    canvas = Image.new("RGBA", (CELL_W*3, CELL_H*2), (20, 27, 37, 255))
    draw = ImageDraw.Draw(canvas)
    for cell, frame_index in enumerate(indexes):
        x, y = (cell % 3) * CELL_W, (cell // 3) * CELL_H
        checker(draw, (x+5, y+5, x+CELL_W-5, y+CELL_H-24))
        try:
            with Image.open(frames[frame_index]) as source:
                image = source.convert("RGBA")
                image.thumbnail((CELL_W-22, CELL_H-43), Image.Resampling.LANCZOS)
                canvas.alpha_composite(image, (x+(CELL_W-image.width)//2,
                                                y+8+(CELL_H-43-image.height)//2))
        except Exception:
            draw.text((x+12, y+12), "preview unavailable", fill=(235, 125, 125), font=font(11))
        duration = durations[frame_index] if frame_index < len(durations) else 0
        draw.text((x+8, y+CELL_H-21), f"frame {frame_index} · {duration} ms",
                  fill=(220, 228, 239), font=font(11))
    target.parent.mkdir(parents=True, exist_ok=True)
    temp = target.with_name(target.stem + ".tmp.webp")
    canvas.convert("RGB").save(temp, "WEBP", quality=80, method=3)
    temp.replace(target)
    return True


def save_swf_animation_preview(frames: list[Path], frame_rate: float, target: Path) -> Path | None:
    if not frames:
        return None
    if len(frames) == 1:
        return target if save_image_preview(frames[0], target) else None
    if preview_is_fresh(target, frames):
        return target
    count = min(40, len(frames))
    indexes = [0] if count == 1 else sorted({round(i * (len(frames) - 1) / (count - 1)) for i in range(count)})
    decoded = []
    for index in indexes:
        try:
            with Image.open(frames[index]) as source:
                frame = source.convert("RGBA")
                frame.thumbnail((600, 400), Image.Resampling.LANCZOS)
                decoded.append(frame.copy())
        except Exception:
            continue
    if not decoded:
        return None
    width = max(frame.width for frame in decoded)
    height = max(frame.height for frame in decoded)
    canvas_frames = []
    for frame in decoded:
        canvas = Image.new("RGBA", (width, height), (0, 0, 0, 0))
        canvas.alpha_composite(frame, ((width-frame.width)//2, (height-frame.height)//2))
        canvas_frames.append(canvas)
    rate = frame_rate if frame_rate > 0 else 12.0
    duration = max(20, round(1000 * len(frames) / rate / len(canvas_frames)))
    target.parent.mkdir(parents=True, exist_ok=True)
    temp = target.with_name(target.stem + ".tmp.webp")
    try:
        canvas_frames[0].save(
            temp, "WEBP", save_all=True, append_images=canvas_frames[1:],
            duration=duration, loop=0, quality=80, method=3,
        )
    except Exception:
        # APNG is broadly supported by current browsers and is Pillow's
        # fallback if this Python build lacks animated-WebP encoding.
        temp = target.with_suffix(".tmp.png")
        target = target.with_suffix(".png")
        canvas_frames[0].save(
            temp, "PNG", save_all=True, append_images=canvas_frames[1:],
            duration=duration, loop=0, optimize=True,
        )
    temp.replace(target)
    return target


def save_image_preview(source: Path, target: Path):
    try:
        with Image.open(source) as im:
            dims = list(im.size)
            if preview_is_fresh(target, [source]):
                return dims
            image = im.convert("RGBA")
            image.thumbnail((600, 400), Image.Resampling.LANCZOS)
            target.parent.mkdir(parents=True, exist_ok=True)
            temp = target.with_name(target.stem + ".tmp.webp")
            image.save(temp, "WEBP", quality=80, method=3)
            temp.replace(target)
            return dims
    except Exception:
        return None


def ensure_native_previews():
    """Use the client's decoders for game texture formats Pillow cannot read."""
    files_dir = CACHE / "files"
    if not files_dir.is_dir():
        return
    NATIVE_PREVIEWS.mkdir(parents=True, exist_ok=True)
    version_file = NATIVE_PREVIEWS / ".decoder-version"
    decoder_version = "2"
    version_changed = not version_file.is_file() or version_file.read_text(encoding="utf-8").strip() != decoder_version
    if version_changed:
        for marker in NATIVE_PREVIEWS.glob("*.png.error"):
            marker.unlink(missing_ok=True)
        version_file.write_text(decoder_version, encoding="utf-8")
    stale = version_changed
    for copies in variants(files_dir).values():
        source = best_variant(copies)
        if magic(source) not in {"JXR", "ATF"}:
            continue
        target = NATIVE_PREVIEWS / (source.name + ".png")
        marker = target.with_name(target.name + ".error")
        if not stale and (not target.is_file() or target.stat().st_mtime_ns < source.stat().st_mtime_ns) and (
            not marker.is_file() or marker.stat().st_mtime_ns < source.stat().st_mtime_ns
        ):
            stale = True
            break
    if not stale:
        return
    try:
        subprocess.run(
            ["cargo", "run", "--quiet", "--bin", "asset-preview", "--",
             str(files_dir), str(NATIVE_PREVIEWS)],
            cwd=ROOT, check=True,
        )
    except FileNotFoundError:
        print("  cargo is unavailable; JPEG-XR / ATF previews will be skipped.", flush=True)
    except subprocess.CalledProcessError as exc:
        print(f"  native texture preview decoder failed (exit {exc.returncode}); continuing.", flush=True)


def missing_preview_reason(kind: str) -> str:
    return {
        "SWF": "Flash library; its exported symbols need SWF rendering. Use Sprite clips for symbols already extracted.",
        "XML / text": "Structured metadata or atlas layout, not a standalone image.",
        "ATF": "Adobe Texture Format; native decoder did not produce a preview for this variant.",
        "JXR": "JPEG-XR image; native decoder did not produce a preview for this variant.",
        "FLV": "Video asset; this catalogue does not extract a still frame.",
        "ZIP": "Archive; contents are not expanded into individual previews.",
        "unknown binary": "Binary asset with no generic image preview. It may be embedded data or a game-specific format.",
        "unreadable": "The cached file could not be read.",
    }.get(kind, "No image preview is available for this file.")


def magic(path: Path) -> str:
    try:
        with path.open("rb") as source:
            head = source.read(32)
    except OSError:
        return "unreadable"
    if head.startswith((b"FWS", b"CWS", b"ZWS")): return "SWF"
    if head.startswith(b"\x89PNG\r\n\x1a\n"): return "PNG"
    if head.startswith(b"\xff\xd8\xff"): return "JPEG"
    if head.startswith((b"GIF87a", b"GIF89a")): return "GIF"
    if head.startswith(b"PK\x03\x04"): return "ZIP"
    if head.startswith(b"ATF"): return "ATF"
    if head.startswith(b"II\xbc\x01") or head.startswith(b"MM\x01\xbc"): return "JXR"
    if head.lstrip().startswith((b"<?xml", b"<")): return "XML / text"
    return "unknown binary"


def collect_swf_inputs():
    inputs = []
    groups = variants(CACHE / "files")
    for name, copies in sorted(groups.items(), key=lambda item: item[0].casefold()):
        source = best_variant(copies)
        if magic(source) == "SWF":
            inputs.append({"key": asset_id("swf", name), "path": str(source.resolve()),
                           "name": name, "kind": "CDN SWF", "cache_path": source.relative_to(ROOT).as_posix()})
    main_dir = CACHE / "main-swf"
    if main_dir.is_dir():
        for source in sorted(main_dir.iterdir(), key=lambda path: path.name.casefold()):
            if source.is_file() and magic(source) == "SWF":
                inputs.append({"key": asset_id("swf", "main:" + source.name),
                               "path": str(source.resolve()), "name": "Main game SWF",
                               "kind": "Main game SWF", "cache_path": source.relative_to(ROOT).as_posix()})
    return inputs


def swf_manifest_current(source: dict) -> bool:
    path = Path(source["path"])
    manifest_path = SWF_RASTER_CACHE / source["key"] / "manifest.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        if manifest.get("source_bytes") != path.stat().st_size or manifest.get("source_modified_ns") != path.stat().st_mtime_ns:
            return False
        return all((manifest_path.parent / frame).is_file()
                   for export in manifest.get("exports", []) for frame in export.get("frames", []))
    except (OSError, json.JSONDecodeError):
        return False


def swf_catalog_entries():
    inputs = collect_swf_inputs()
    if not inputs:
        return [], [], {}
    SWF_RASTER_CACHE.mkdir(parents=True, exist_ok=True)
    SWF_INPUTS.write_text(json.dumps([{"key": item["key"], "path": item["path"]} for item in inputs]),
                          encoding="utf-8")
    stale = [item for item in inputs if not swf_manifest_current(item)]
    if stale:
        print(f"Rasterizing exported symbols in {len(stale)}/{len(inputs)} SWFs…", flush=True)
        try:
            subprocess.run(
                ["cargo", "run", "--quiet", "--bin", "swf-catalog", "--",
                 str(SWF_INPUTS), str(SWF_RASTER_CACHE)], cwd=ROOT, check=True,
            )
        except FileNotFoundError:
            print("  cargo is unavailable; SWF sources will be listed without rendered symbols.", flush=True)
        except subprocess.CalledProcessError as exc:
            print(f"  SWF rasterizer failed (exit {exc.returncode}); using available cached renders.", flush=True)

    sources, symbols, previews_by_name = [], [], {}
    preview_jobs = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        for source in inputs:
            manifest_path = SWF_RASTER_CACHE / source["key"] / "manifest.json"
            try:
                manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError):
                manifest = {"exports": [], "parse_error": "SWF has not been rasterized yet."}
            export_names = []
            source_symbols = []
            rate = float(manifest.get("frame_rate") or 12.0)
            for export_index, export in enumerate(manifest.get("exports", [])):
                name = str(export.get("name", "(unnamed)"))
                export_names.append(name)
                frame_paths = [manifest_path.parent / item for item in export.get("frames", [])]
                target = OUT / "previews" / "swf" / f"{source['key']}-{export_index:04d}.webp"
                row = {
                    "id": asset_id("swfsymbol", source["key"] + "\0" + name),
                    "source_id": source["key"], "source_name": source["name"],
                    "source_kind": source["kind"], "name": name,
                    "frames": len(frame_paths), "frame_rate": rate,
                    "preview": None,
                    "frames_path": frame_paths[0].parent.relative_to(ROOT).as_posix() if frame_paths else None,
                    "cache_path": manifest_path.parent.relative_to(ROOT).as_posix(),
                    "preview_reason": export.get("error") or "The exported symbol did not render a frame.",
                }
                symbols.append(row)
                source_symbols.append(row)
                if frame_paths:
                    future = pool.submit(save_swf_animation_preview, frame_paths, rate, target)
                    preview_jobs[future] = (row, source["key"])
            sources.append({
                "id": source["key"], "name": source["name"], "kind": source["kind"],
                "bytes": Path(source["path"]).stat().st_size,
                "cache_path": source["cache_path"], "symbol_count": len(export_names),
                "rendered_symbols": 0, "symbols": export_names,
                "preview": None, "preview_reason": manifest.get("parse_error") or "No exported symbol rendered successfully.",
                "_symbol_rows": source_symbols,
            })

        job_count = len(preview_jobs)
        for completed, future in enumerate(concurrent.futures.as_completed(preview_jobs), 1):
            row, _ = preview_jobs[future]
            try:
                rendered_preview = future.result()
            except Exception:
                rendered_preview = None
            if rendered_preview:
                row["preview"] = rendered_preview.relative_to(OUT).as_posix()
                row["preview_reason"] = None
            if completed % 100 == 0 or completed == job_count:
                print(f"  SWF symbol previews {completed}/{job_count}", flush=True)

    for source_row in sources:
        source_symbols = source_row.pop("_symbol_rows")
        successful = [symbol["preview"] for symbol in source_symbols if symbol["preview"]]
        animated = [symbol["preview"] for symbol in source_symbols
                    if symbol["preview"] and symbol["frames"] > 1]
        source_row["rendered_symbols"] = len(successful)
        source_row["preview"] = (animated or successful)[0] if successful else None
        source_row["preview_reason"] = None if successful else source_row["preview_reason"]
        if source_row["kind"] == "CDN SWF":
            previews_by_name[source_row["name"]] = (source_row["preview"], source_row["preview_reason"])
    return sources, symbols, previews_by_name


def raw_file_entries(swf_previews_by_name=None):
    swf_previews_by_name = swf_previews_by_name or {}
    groups = variants(CACHE / "files")
    entries, path_by_id = [], {}
    ordered = sorted(groups.items(), key=lambda x: x[0].casefold())
    image_count = 0
    for index, (name, copies) in enumerate(ordered, 1):
        if index == 1 or index % 500 == 0 or index == len(ordered):
            print(f"  raw files indexed: {index}/{len(ordered)} ({image_count} image previews)", flush=True)
        path = best_variant(copies)
        path_by_id[name] = path
        kind = magic(path)
        preview = None
        dims = None
        if kind in {"PNG", "JPEG", "GIF"}:
            pid = asset_id("raw", name)
            preview = f"previews/{pid}.webp"
            dims = save_image_preview(path, OUT / preview)
            image_count += 1
        elif kind in {"JXR", "ATF"}:
            native = NATIVE_PREVIEWS / (path.name + ".png")
            if native.is_file():
                pid = asset_id("raw", name)
                preview = f"previews/{pid}.webp"
                dims = save_image_preview(native, OUT / preview)
                if dims:
                    image_count += 1
        elif kind == "SWF":
            preview, _ = swf_previews_by_name.get(name, (None, None))
            if preview:
                image_count += 1
        entries.append({
            "id": asset_id("file", name), "name": name, "kind": kind,
            "bytes": path.stat().st_size, "variant_count": len(copies),
            "cache_path": path.relative_to(ROOT).as_posix(),
            "preview": preview, "dimensions": dims,
            "preview_reason": None if preview else (
                swf_previews_by_name.get(name, (None, "SWF has not been rasterized yet."))[1]
                if kind == "SWF" else missing_preview_reason(kind)
            ),
        })
    by_name = {entry["name"]: entry for entry in entries}
    for entry in entries:
        if entry["kind"] != "ATF" or entry["preview"] or not entry["name"].endswith("_atf"):
            continue
        companion = by_name.get(entry["name"][:-4])
        if companion and companion.get("preview"):
            entry["preview"] = companion["preview"]
            entry["dimensions"] = companion.get("dimensions")
            entry["preview_reason"] = None
            entry["preview_source"] = companion["name"]
    return entries, path_by_id


def atlas_entries(path_by_id: dict[str, Path], swf_symbols=None):
    swf_symbols = swf_symbols or []
    result = []
    atlas_paths = [p for name, p in path_by_id.items() if name.endswith(("_atlasXml", "_atlas"))]
    atlas_paths = sorted(atlas_paths, key=lambda p: p.name.casefold())
    for atlas_index, xml_path in enumerate(atlas_paths, 1):
        if atlas_index == 1 or atlas_index % 10 == 0 or atlas_index == len(atlas_paths):
            print(f"  atlas sheets: {atlas_index}/{len(atlas_paths)}", flush=True)
        try:
            root = ET.parse(xml_path).getroot()
        except (ET.ParseError, OSError):
            continue
        if root.tag.split("}")[-1] != "TextureAtlas":
            continue
        name = re.sub(r"__(?:h[0-9a-fA-F]*|v[^/]+)$", "", xml_path.name)
        texture_name = Path(root.get("imagePath", "")).stem
        atlas_base = re.sub(r"_atlas(?:Xml)?$", "", name)
        related_symbols = [symbol for symbol in swf_symbols
                           if symbol.get("source_name", "").casefold() == atlas_base.casefold()]
        # The feature-menu atlas carries icon glyphs only. The button frame is
        # vector art exported by the matching SWF, so compose the sheet over
        # that actual game symbol while retaining the source symbol below.
        button_background = next((symbol for symbol in related_symbols
                                  if symbol.get("name", "").casefold() in {"featurebtn", "featurebutton"}
                                  and symbol.get("frames_path")), None)
        button_background_path = None
        if button_background:
            candidate = ROOT / button_background["frames_path"] / "f0001.png"
            if candidate.is_file():
                button_background_path = candidate
        texture_candidates = [texture_name, texture_name + "_texture", atlas_base + "_texture"]
        existing = [key for key in dict.fromkeys(texture_candidates) if key and key in path_by_id]
        matched_texture = next((key for key in existing
                                if magic(path_by_id[key]) in {"PNG", "JPEG", "GIF"}),
                               existing[0] if existing else None)
        texture_path = path_by_id.get(matched_texture) if matched_texture else None
        nodes = root.findall(".//SubTexture")
        regions = [{"name": node.get("name", "(unnamed)"),
                    "sheet": i // ATLAS_PAGE_SIZE}
                   for i, node in enumerate(nodes)]
        sheets = []
        if texture_path and nodes:
            page_jobs = []
            sheet_refs = {}
            for start in range(0, len(nodes), ATLAS_PAGE_SIZE):
                page_no = start // ATLAS_PAGE_SIZE
                composition = "-button-composed" if button_background_path else ""
                rel = f"previews/{asset_id('atlas', name)}-{page_no+1:02d}{composition}.webp"
                target = OUT / rel
                target.parent.mkdir(parents=True, exist_ok=True)
                if preview_is_fresh(target, [xml_path, texture_path]):
                    sheet_refs[page_no] = rel
                else:
                    page_jobs.append((start, page_no, rel, target))
            try:
                background_frame = None
                if button_background_path:
                    with Image.open(button_background_path) as source:
                        background_frame = source.convert("RGBA")
                if page_jobs:
                    with Image.open(texture_path) as src:
                        texture = src.convert("RGBA")
                        for start, page_no, rel, target in page_jobs:
                            page = Image.new("RGBA", (ATLAS_COLS*160, ATLAS_ROWS*122), (22, 29, 40, 255))
                            draw = ImageDraw.Draw(page)
                            for index, node in enumerate(nodes[start:start+ATLAS_PAGE_SIZE]):
                                try:
                                    x, y = int(float(node.get("x", "0"))), int(float(node.get("y", "0")))
                                    w, h = int(float(node.get("width", "0"))), int(float(node.get("height", "0")))
                                    crop = texture.crop((x, y, x+w, y+h))
                                    if node.get("rotated", "false").lower() == "true":
                                        crop = crop.rotate(90, expand=True)
                                    col, row = index % ATLAS_COLS, index // ATLAS_COLS
                                    cx, cy = col*160, row*122
                                    checker(draw, (cx+4, cy+4, cx+156, cy+91), 10)
                                    if background_frame:
                                        tile = Image.new("RGBA", (86, 86), (22, 29, 40, 255))
                                        button = background_frame.copy()
                                        button.thumbnail((86, 86), Image.Resampling.LANCZOS)
                                        tile.alpha_composite(button, ((86-button.width)//2, (86-button.height)//2))
                                        crop.thumbnail((42, 42), Image.Resampling.LANCZOS)
                                        tile.alpha_composite(crop, ((86-crop.width)//2, (86-crop.height)//2))
                                        crop = tile
                                    else:
                                        crop.thumbnail((142, 84), Image.Resampling.LANCZOS)
                                    page.alpha_composite(crop, (cx+80-crop.width//2, cy+47-crop.height//2))
                                    region_name = node.get("name", "(unnamed)")
                                    label = region_name if len(region_name) <= 24 else region_name[:21]+"…"
                                    draw.text((cx+5, cy+96), label, fill=(230, 236, 246), font=font(10))
                                except (ValueError, OSError):
                                    pass
                            temp = target.with_name(target.stem + ".tmp.webp")
                            page.convert("RGB").save(temp, "WEBP", quality=80, method=3)
                            temp.replace(target)
                            sheet_refs[page_no] = rel
                sheets = [sheet_refs[i] for i in sorted(sheet_refs)]
            except Exception:
                sheets = [sheet_refs[i] for i in sorted(sheet_refs)]
                texture_path = None
        result.append({
            "id": asset_id("atlas", name), "name": name,
            "texture": matched_texture or texture_name or None,
            "region_count": len(regions), "regions": regions, "sheets": sheets,
            "related_swf_symbols": related_symbols,
            "xml_path": xml_path.relative_to(ROOT).as_posix(),
            "texture_path": texture_path.relative_to(ROOT).as_posix() if texture_path else None,
            "preview_reason": None if sheets else "Atlas metadata was found, but its referenced texture was not cached or could not be decoded.",
            "search": " ".join(r["name"] for r in regions),
        })
    return result


def sprite_entries():
    grouped: dict[tuple[str, str], list[Path]] = defaultdict(list)
    for meta in (CACHE / "sprites").rglob("meta.json"):
        relative = meta.relative_to(CACHE / "sprites")
        if len(relative.parts) >= 4:
            grouped[(relative.parts[0], relative.parts[1])].append(meta)
    clips = []
    ordered = sorted(grouped.items(), key=lambda x: (x[0][0].casefold(), x[0][1].casefold()))
    for index, ((source, resource), metas) in enumerate(ordered, 1):
        if index == 1 or index % 100 == 0 or index == len(ordered):
            print(f"  sprite previews: {index}/{len(ordered)}", flush=True)
        latest = max(metas, key=lambda p: (version_number(p), p.stat().st_mtime_ns))
        try:
            meta = json.loads(latest.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            meta = {}
        frame_meta = meta.get("frames", []) if isinstance(meta, dict) else []
        frames = sorted(latest.parent.glob("f*.png"),
                        key=lambda p: int(re.search(r"f(\d+)", p.stem).group(1))
                        if re.search(r"f(\d+)", p.stem) else 0)
        durations = [int(x.get("duration_ms", 0) or 0) for x in frame_meta]
        frame_nums = [int(re.search(r"f(\d+)", p.stem).group(1))
                      if re.search(r"f(\d+)", p.stem) else i for i, p in enumerate(frames)]
        cid = asset_id("clip", source + "\0" + resource)
        preview = f"previews/{cid}.webp"
        if not save_clip_preview(frames, durations, OUT / preview):
            preview = None
        clips.append({
            "id": cid, "source": source, "resource": resource, "frames": len(frames),
            "duration_ms": sum(durations), "rotation_strip": bool(meta.get("rotation_strip", False)),
            "tiles": bool(meta.get("tiles", False)),
            "versions": sorted({version_number(p) for p in metas}),
            "version": version_number(latest), "frame_numbers": frame_nums, "preview": preview,
            "cache_path": latest.parent.relative_to(ROOT).as_posix(),
        })
    return clips


def other_cache_entries():
    entries = []
    lang_dir = CACHE / "lang"
    if lang_dir.exists():
        for path in sorted(lang_dir.rglob("*.xml")):
            entries.append({"name": f"Locale {path.relative_to(lang_dir).as_posix()}",
                            "kind": "XML / locale", "bytes": path.stat().st_size,
                            "path": path.relative_to(ROOT).as_posix()})
    return entries


HTML = r"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Xenomit client asset catalogue</title>
<style>
:root{color-scheme:dark;--bg:#0a111c;--panel:#111d2b;--line:#24364a;--text:#e2ebf6;--muted:#99abc0;--blue:#68b7ff;--gold:#f5c45c}
*{box-sizing:border-box}body{margin:0;background:radial-gradient(ellipse at 50% -20%,#1d3550 0,var(--bg) 52%);color:var(--text);font:14px/1.45 "Segoe UI",system-ui,sans-serif}
header{position:sticky;top:0;z-index:2;padding:18px clamp(16px,4vw,48px) 14px;background:#0b1420ee;border-bottom:1px solid var(--line);backdrop-filter:blur(12px)}
h1{font-size:22px;margin:0 0 4px}header p{margin:0;color:var(--muted)}.scope{margin:12px 0 0;padding:9px 12px;border-left:3px solid var(--gold);background:#2b271b;color:#e7d9ae}
main{padding:18px clamp(12px,3vw,36px) 60px}.toolbar{display:flex;gap:10px;align-items:center;flex-wrap:wrap;margin-bottom:14px}input{flex:1;min-width:260px;background:#101c29;color:var(--text);border:1px solid var(--line);border-radius:8px;padding:11px 13px;font:inherit}.tabs{display:flex;gap:6px;flex-wrap:wrap}.tab{border:1px solid var(--line);background:#101c29;color:var(--muted);border-radius:7px;padding:8px 11px;cursor:pointer}.tab.active{color:#07121f;background:var(--blue);border-color:var(--blue);font-weight:700}.summary{color:var(--muted);margin:7px 0 16px}.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(270px,1fr));gap:12px}.card{min-width:0;border:1px solid var(--line);border-radius:10px;background:linear-gradient(160deg,#152437,var(--panel) 60%);overflow:hidden}.card img{display:block;width:100%;height:auto;background:#101923;border-bottom:1px solid var(--line)}.card-body{padding:12px}.title{font-weight:700;font-size:15px;overflow-wrap:anywhere}.badge{display:inline-block;margin:5px 5px 0 0;padding:2px 7px;border-radius:999px;background:#1d3145;color:#acd6fb;font-size:11px}.meta{margin-top:8px;color:var(--muted);font-size:12px;overflow-wrap:anywhere}.kv{margin-top:7px;display:grid;grid-template-columns:74px 1fr;gap:3px 7px;font-size:12px}.kv span:nth-child(odd){color:var(--muted)}code{color:#c4ddf8;overflow-wrap:anywhere}.copy{margin-top:9px;border:1px solid #385573;border-radius:6px;background:#12263a;color:#cbe6ff;padding:5px 8px;cursor:pointer}details{margin-top:9px;border-top:1px solid var(--line);padding-top:7px}summary{cursor:pointer;color:#b9d7f2}.regions{display:flex;gap:4px;flex-wrap:wrap;margin-top:8px}.region{font:11px/1.2 ui-monospace,monospace;color:#d8e5f4;background:#1b2b3b;padding:3px 5px;border-radius:4px}.sheets{display:flex;gap:6px;flex-wrap:wrap;margin-top:8px}.sheets img{width:min(100%,320px);height:auto;border:1px solid var(--line);border-radius:5px}.empty{padding:40px;text-align:center;color:var(--muted);border:1px dashed var(--line);border-radius:10px}.footer{color:var(--muted);font-size:12px;margin-top:30px}
</style></head><body><header><h1>Xenomit client asset catalogue</h1><p id="stats"></p><div class="scope">This catalogue is generated from assets present in the local client cache. It is not a complete listing of uncached files still available on the game CDN. Run <code>python tools/build_asset_catalog.py</code> again after playing or loading more screens to refresh it.</div></header>
<main><div class="toolbar"><input id="search" type="search" placeholder="Search asset ID, clip/resource key, atlas region, file type…" autocomplete="off"><div class="tabs" id="tabs"></div></div><div class="summary" id="summary"></div><div class="grid" id="grid"></div><div class="footer">Use the displayed asset ID and source/resource pair when asking for an implementation. Atlas entries include searchable region names and labelled sprite sheets. Cache paths point to the original local data.</div></main>
<script id="catalog-data" type="application/json">__DATA__</script>
<script>
const D=JSON.parse(document.getElementById('catalog-data').textContent), search=document.getElementById('search'), grid=document.getElementById('grid'), summary=document.getElementById('summary'), tabs=document.getElementById('tabs');
const categories=[['all','Everything'],['cdn','CDN index'],['clips','Sprite clips'],['atlases','Atlas regions'],['swf-originals','Original SWFs'],['swf-rasterized','Rasterized SWF symbols'],['files','Raw files'],['other','Locale']];let active='all',limit=120;
document.getElementById('stats').textContent=D.counts.resource_entries+' CDN index entries · '+D.counts.clips+' processed sprite clips · '+D.counts.atlas_regions+' atlas regions · '+D.counts.swf_sources+' original SWFs · '+D.counts.swf_symbols+' rasterized symbols · '+D.counts.previewed_files+'/'+D.counts.files+' cached file previews · generated '+D.generated;
for(const [id,label] of categories){const b=document.createElement('button');b.className='tab'+(id===active?' active':'');b.textContent=label;b.dataset.id=id;b.onclick=()=>{active=id;for(const x of tabs.children)x.classList.toggle('active',x.dataset.id===id);render()};tabs.append(b)}
function el(tag,cls,txt){const n=document.createElement(tag);if(cls)n.className=cls;if(txt!==undefined)n.textContent=txt;return n}
function kv(parent,rows){const d=el('div','kv');for(const [k,v] of rows)d.append(el('span','',k),el('code','',String(v)));parent.append(d)}
function copyButton(parent,text){const b=el('button','copy','Copy asset reference');b.onclick=async()=>{try{await navigator.clipboard.writeText(text);b.textContent='Copied'}catch{b.textContent=text}};parent.append(b)}
function cardBase(name,badges,preview,note){const c=el('article','card');if(preview){const im=el('img');im.loading='lazy';im.src=preview;im.alt=name;c.append(im)}const body=el('div','card-body');body.append(el('div','title',''+name));for(const x of badges)body.append(el('span','badge',x));if(!preview&&note)body.append(el('div','meta',note));c.append(body);return[c,body]}
function clipCard(a){const [c,b]=cardBase(a.source+' / '+a.resource,[a.frames+' frames',a.rotation_strip?'rotation strip':'animation/static','v'+a.version],a.preview);kv(b,[['source',a.source],['resource',a.resource],['timeline',a.duration_ms+' ms'],['versions',a.versions.join(', ')],['cache',a.cache_path]]);copyButton(b,'source='+a.source+'; resource='+a.resource);return c}
function atlasCard(a,q){const [c,b]=cardBase(a.name,[a.region_count+' regions',a.texture||'texture not cached'],a.sheets[0],a.preview_reason);kv(b,[['atlas XML',a.name],['texture',a.texture||'not found'],['cache',a.xml_path]]);const matches=a.regions.filter(r=>!q||r.name.toLowerCase().includes(q)),d=el('details');d.append(el('summary','',matches.length+(q?' matching':'')+' of '+a.region_count+' region names'));const wrap=el('div','sheets'),needed=new Set(matches.map(r=>r.sheet));if(!q)for(let i=1;i<a.sheets.length;i++)needed.add(i);for(const i of [...needed].sort((x,y)=>x-y)){if(!a.sheets[i])continue;const im=el('img');im.loading='lazy';im.src=a.sheets[i];im.alt=a.name+' regions sheet '+(i+1);wrap.append(im)}d.append(wrap);const chips=el('div','regions');for(const r of matches){const n=el('span','region',r.name);n.title='Sheet '+(r.sheet+1);chips.append(n)}d.append(chips);b.append(d);if(a.related_swf_symbols?.length){const related=el('details');related.append(el('summary','','Related SWF symbols ('+a.related_swf_symbols.length+')'));const refs=el('div','sheets');for(const symbol of a.related_swf_symbols){const item=el('div','');item.style='width:140px;color:var(--muted);font-size:12px';if(symbol.preview){const im=el('img');im.loading='lazy';im.src=symbol.preview;im.alt=symbol.source_name+' / '+symbol.name;item.append(im)}item.append(el('div','',symbol.name+' · '+symbol.frames+' frame'+(symbol.frames===1?'':'s')));refs.append(item)}related.append(refs);b.append(related)}copyButton(b,'atlas='+a.name+'; texture='+(a.texture||'')+'; region='+(matches[0]?.name||''));return c}
function swfOriginalCard(a,q){const [c,b]=cardBase(a.name,[a.kind,a.bytes.toLocaleString()+' bytes',a.rendered_symbols+'/'+a.symbol_count+' symbols rendered'],a.preview,a.preview_reason);kv(b,[['original',a.cache_path],['rasterized symbols',a.rendered_symbols],['symbol exports',a.symbol_count]]);const names=a.symbols.filter(n=>!q||n.toLowerCase().includes(q)),d=el('details');d.append(el('summary','',Math.min(names.length,120)+(names.length>120?' shown':'')+' of '+names.length+' symbol names'));const chips=el('div','regions');for(const n of names.slice(0,120))chips.append(el('span','region',n));d.append(chips);b.append(d);copyButton(b,'swf='+a.id+'; original='+a.cache_path);return c}
function swfRasterCard(a){const [c,b]=cardBase(a.source_name+' / '+a.name,[a.frames+' frame'+(a.frames===1?'':'s'),a.frame_rate.toFixed(1)+' fps',a.source_kind],a.preview,a.preview_reason);kv(b,[['original SWF',a.source_name],['symbol',a.name],['raster frames',a.frames],['frame cache',a.frames_path||a.cache_path]]);copyButton(b,'swf='+a.source_id+'; symbol='+a.name);return c}
function fileCard(a){const badges=[a.kind,a.bytes.toLocaleString()+' bytes',a.variant_count+' cached version'+(a.variant_count===1?'':'s')];if(a.preview_source)badges.push('preview paired from '+a.preview_source);const [c,b]=cardBase(a.name,badges,a.preview,a.preview_reason);kv(b,[['cache',a.cache_path],['dimensions',a.dimensions?a.dimensions.join(' × '):'n/a']]);copyButton(b,'file='+a.name);return c}
function otherCard(a){const [c,b]=cardBase(a.name,[a.kind,a.bytes.toLocaleString()+' bytes'],null);kv(b,[['cache',a.path]]);return c}
function resourceCard(a){const status=!a.cached?'not downloaded':a.cache_bytes===0?'empty file from CDN':'cached';const [c,b]=cardBase(a.id,[a.kind,status],a.preview,a.preview_reason);kv(b,[['CDN path',a.path+a.name+'.'+a.type],['version',a.version||'—'],['hash',a.hash||'—'],['cache',a.cache_path||'not downloaded']]);copyButton(b,'file='+a.id+'; type='+a.type+'; location='+a.path);return c}
function render(){const q=search.value.trim().toLowerCase();grid.replaceChildren();if(grid.nextElementSibling?.classList.contains('more'))grid.nextElementSibling.remove();let rows=[];if(active==='cdn')rows=D.resources.filter(x=>(x.id+' '+x.path+' '+x.name+' '+x.type+' '+x.version+' '+x.hash).toLowerCase().includes(q)).map(x=>[resourceCard,x]);else if(active==='swf-originals')rows=D.swf_sources.filter(x=>(x.id+' '+x.name+' '+x.cache_path+' '+x.symbols.join(' ')).toLowerCase().includes(q)).map(x=>[a=>swfOriginalCard(a,q),x]);else if(active==='swf-rasterized')rows=D.swf_symbols.filter(x=>(x.id+' '+x.source_name+' '+x.name+' '+x.source_id).toLowerCase().includes(q)).map(x=>[swfRasterCard,x]);else{if(active==='all'||active==='clips')rows.push(...D.clips.filter(x=>(x.source+' '+x.resource+' '+x.frames+' '+x.cache_path).toLowerCase().includes(q)).map(x=>[clipCard,x]));if(active==='all'||active==='atlases')rows.push(...D.atlases.filter(x=>(x.name+' '+x.texture+' '+x.search).toLowerCase().includes(q)).map(x=>[a=>atlasCard(a,q),x]));if(active==='all'||active==='files')rows.push(...D.files.filter(x=>x.kind!=='SWF'&&(x.name+' '+x.kind+' '+x.cache_path).toLowerCase().includes(q)).map(x=>[fileCard,x]));if(active==='all'||active==='other')rows.push(...D.other.filter(x=>(x.name+' '+x.kind+' '+x.path).toLowerCase().includes(q)).map(x=>[otherCard,x]))}const total=rows.length;for(const [make,item] of rows.slice(0,limit))grid.append(make(item));summary.textContent='Showing '+Math.min(total,limit)+' of '+total+' entries'+(q?' matching "'+search.value.trim()+'"':'')+'.';if(total>limit){const more=el('button','more','Load more entries');more.onclick=()=>{limit+=120;render()};grid.after(more)}if(!total)grid.append(el('div','empty','No entries match that search.'))}
search.addEventListener('input',()=>{limit=120;render()});render();
</script></body></html>"""


def main():
    parser = argparse.ArgumentParser(description="Generate the local asset catalogue.")
    parser.add_argument(
        "--download-all", action="store_true",
        help="download every file listed in the last client-fetched resources.xml",
    )
    args = parser.parse_args()
    if not CACHE.exists():
        raise SystemExit(f"Asset cache does not exist: {CACHE}")
    source_metadata, source_entries = load_cdn_index()
    if args.download_all:
        download_all_resources(source_metadata, source_entries)
    OUT.mkdir(parents=True, exist_ok=True)
    PREVIEWS.mkdir(parents=True, exist_ok=True)
    print("Rebuilding catalogue from cached client assets…", flush=True)
    ensure_native_previews()
    print("Preparing separate original and rasterized SWF catalogues…", flush=True)
    swf_sources, swf_symbols, swf_previews_by_name = swf_catalog_entries()
    files, path_by_id = raw_file_entries(swf_previews_by_name)
    resources = indexed_resource_entries(source_metadata, source_entries, path_by_id, files)
    print("Building atlas-region previews…", flush=True)
    atlases = atlas_entries(path_by_id, swf_symbols)
    print("Updating processed sprite previews…", flush=True)
    clips = sprite_entries()
    other = other_cache_entries()
    catalog = {
        "generated": datetime.now(timezone.utc).astimezone().strftime("%Y-%m-%d %H:%M %Z"),
        "counts": {
            "clips": len(clips), "atlases": len(atlases),
            "atlas_regions": sum(x["region_count"] for x in atlases),
            "files": len(files), "other": len(other),
            "resource_entries": len(resources),
            "uncached_resource_entries": sum(not x["cached"] for x in resources),
            "previewed_files": sum(bool(x["preview"]) for x in files),
            "swf_sources": len(swf_sources), "swf_symbols": len(swf_symbols),
        },
        "resource_index": source_metadata,
        "resources": resources,
        "clips": clips, "atlases": atlases, "files": files, "other": other,
        "swf_sources": swf_sources, "swf_symbols": swf_symbols,
    }
    (OUT / "catalog.json").write_text(json.dumps(catalog, ensure_ascii=False, indent=2), encoding="utf-8")
    payload = json.dumps(catalog, ensure_ascii=False, separators=(",", ":"))
    payload = payload.replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")
    html_text = HTML.replace("__DATA__", payload)
    html_text = html_text.replace(
        "This catalogue is generated from assets present in the local client cache. It is not a complete listing of uncached files still available on the game CDN. Run <code>python tools/build_asset_catalog.py</code> again after playing or loading more screens to refresh it.",
        "The CDN tab lists every file in the latest resource index fetched during client startup. Run <code>python tools/build_asset_catalog.py --download-all</code> to download missing files and create previews. This can use substantial bandwidth and disk space. The index is refreshed when the client next reaches the game.",
    )
    html_text = html_text.replace(
        "</style>",
        ".more{display:block;margin:18px auto;border:1px solid #385573;border-radius:7px;background:#12263a;color:#cbe6ff;padding:9px 14px;cursor:pointer}</style>",
    )
    (OUT / "index.html").write_text(html_text, encoding="utf-8")
    print("Catalogue generated:")
    print(f"  sprite clips: {len(clips)}")
    print(f"  atlases: {len(atlases)} ({catalog['counts']['atlas_regions']} regions)")
    print(f"  raw cached files: {len(files)} unique IDs")
    print(f"  original SWFs: {len(swf_sources)}; rasterized symbols: {len(swf_symbols)}")
    print(f"  CDN index entries: {len(resources)} ({catalog['counts']['uncached_resource_entries']} not cached)")
    print(f"  locale / SWF files: {len(other)}")
    print(f"  output: {OUT}")


if __name__ == "__main__":
    main()
