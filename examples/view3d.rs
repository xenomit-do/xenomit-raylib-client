//! `view3d [out.png]` — offline check of the 3D pipeline. Renders cached
//! `aegis` (resources_3d) in several headings over a map-space grid with the
//! real camera, plus a 2D overlay drawn through `Camera::to_screen` so the two
//! projections can be compared by eye.
use raylib::prelude::*;
use xenomit_client::assets::{atf, awd};
use xenomit_client::game::Camera as Cam2D;
use xenomit_client::render::scene3d;

fn cached(prefix: &str) -> Vec<u8> {
    let dir = ".xenomit-assets/resources_3d/files";
    let entry = std::fs::read_dir(dir)
        .expect("cache dir")
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .unwrap_or_else(|| panic!("no cached {prefix}"));
    std::fs::read(entry.path()).unwrap()
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "view3d.png".into());
    let perspective = std::env::args().nth(2).as_deref() != Some("ortho");
    let (mut rl, thread) = raylib::init().size(1280, 720).title("view3d").build();
    rl.set_target_fps(60);

    // `geometry:texture` pairs (cache file prefixes) after the 2 positional args.
    let pairs: Vec<(String, String)> = std::env::args()
        .skip(3)
        .filter_map(|a| a.split_once(':').map(|(g, t)| (g.to_owned(), t.to_owned())))
        .collect();
    let pairs = if pairs.is_empty() {
        vec![("aegis".to_owned(), "aegis_diffuse_128".to_owned())]
    } else {
        pairs
    };
    let mut models: Vec<(Model, Texture2D)> = Vec::new();
    for (i, (g, t)) in pairs.iter().enumerate() {
        let mesh = awd::parse_aw2(&cached(&format!("{g}__"))).expect("awd");
        let obj = std::env::temp_dir().join(format!("view3d-{i}.obj"));
        std::fs::write(&obj, mesh.to_obj()).unwrap();
        let mut model = rl
            .load_model(&thread, &obj.to_string_lossy())
            .expect("model");
        let image = atf::decode(&cached(&format!("{t}__"))).expect("atf");
        let (w, h) = image.dimensions();
        let tex = unsafe {
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
            rl.load_texture_from_image(&thread, &img).expect("texture")
        };
        for m in model.materials_mut() {
            m.set_material_texture(MaterialMapIndex::MATERIAL_MAP_ALBEDO, &tex);
        }
        models.push((model, tex));
    }
    let model = &models[0].0;

    // UberPlane3D check: asymmetric glyph on the shared plane model.
    let plane_tex = {
        let mut img = Image::gen_image_color(256, 256, Color::new(30, 60, 30, 255));
        img.draw_text("F", 70, 20, 220, Color::WHITE);
        img.draw_rectangle(0, 0, 256, 12, Color::RED); // top edge red
        rl.load_texture_from_image(&thread, &img)
            .expect("plane tex")
    };
    let mut plane = {
        let mesh = Mesh::gen_mesh_plane(&thread, 1.0, 1.0, 1, 1);
        rl.load_model_from_mesh(&thread, unsafe { mesh.make_weak() })
            .expect("plane")
    };
    plane.set_transform(&Matrix::rotate_x(std::f32::consts::FRAC_PI_2));
    for m in plane.materials_mut() {
        m.set_material_texture(MaterialMapIndex::MATERIAL_MAP_ALBEDO, &plane_tex);
    }
    let bar_tex = {
        let mut img = Image::gen_image_color(64, 8, Color::RED);
        img.draw_rectangle(48, 0, 16, 8, Color::WHITE); // white tip on the +x end
        rl.load_texture_from_image(&thread, &img).expect("bar tex")
    };
    let center = (5000.0f32, 5000.0f32);
    let cam2d = Cam2D {
        center,
        zoom: 1.0,
        viewport: (1280.0, 720.0),
        perspective,
    };
    // Recipe: aegis mesh scale 0.7-ish; headings are the game's
    // `rotation.degrees` (travel angle + 180).
    let ships = [
        ((5000.0, 5000.0), 180.0, "E"), // travelling east
        ((5300.0, 5000.0), 270.0, "S"), // travelling south (+map y)
        ((4700.0, 5000.0), 0.0, "W"),
        ((5000.0, 4700.0), 90.0, "N"),
    ];
    for frame in 0..3 {
        let camera = scene3d::orbit_camera(center, 1.0, 720.0, perspective);
        let mut d = rl.begin_drawing(&thread);
        d.clear_background(Color::new(10, 12, 24, 255));
        {
            let mut m3 = d.begin_mode3D(camera);
            for i in -10..=10 {
                let t = i as f32 * 100.0;
                let col = if i == 0 {
                    Color::GRAY
                } else {
                    Color::new(40, 40, 70, 255)
                };
                m3.draw_line_3D(
                    scene3d::rl_point(4000.0, 0.0, -(5000.0 + t)),
                    scene3d::rl_point(6000.0, 0.0, -(5000.0 + t)),
                    col,
                );
                m3.draw_line_3D(
                    scene3d::rl_point(5000.0 + t, 0.0, -4000.0),
                    scene3d::rl_point(5000.0 + t, 0.0, -6000.0),
                    col,
                );
            }
            // East (+x) red, south (+map y) blue.
            m3.draw_line_3D(
                scene3d::rl_point(5000.0, 1.0, -5000.0),
                scene3d::rl_point(5400.0, 1.0, -5000.0),
                Color::RED,
            );
            m3.draw_line_3D(
                scene3d::rl_point(5000.0, 1.0, -5000.0),
                scene3d::rl_point(5000.0, 1.0, -5400.0),
                Color::BLUE,
            );
            {
                // Authored background plane: rotationX=45, facing the camera.
                let (axis, angle) = scene3d::euler_axis_angle((45.0, 0.0, 0.0));
                m3.draw_model_ex(
                    &plane,
                    scene3d::rl_point(5000.0, 0.0, -4500.0),
                    axis,
                    angle,
                    Vector3::new(250.0, 250.0, 250.0),
                    Color::WHITE,
                );
            }
            {
                // Laser billboard check: bar along the line (5000,5000)->(5250,4850).
                let (a, b) = ((5000.0f32, 5000.0f32), (5250.0f32, 4850.0f32));
                m3.draw_line_3D(
                    scene3d::rl_point(a.0, 2.0, -a.1),
                    scene3d::rl_point(b.0, 2.0, -b.1),
                    Color::GREEN,
                );
                let (sa, sb) = (cam2d.to_screen(a), cam2d.to_screen(b));
                let angle = (sb.1 - sa.1).atan2(sb.0 - sa.0).to_degrees();
                let mid = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
                let len = ((sb.0 - sa.0).hypot(sb.1 - sa.1) / cam2d.scale_at(mid)) as f32;
                m3.draw_billboard_pro(
                    camera,
                    *bar_tex.as_ref(),
                    Rectangle::new(0.0, 0.0, 64.0, 8.0),
                    scene3d::rl_point(mid.0, 2.0, -mid.1),
                    Vector3::new(0.0, 1.0, 0.0),
                    Vector2::new(len, 8.0),
                    Vector2::new(len * 0.5, 4.0),
                    -angle,
                    Color::new(255, 255, 255, 160),
                );
            }
            if pairs.len() > 1 {
                for (i, (m, _)) in models.iter().enumerate() {
                    let pos = (
                        4400.0 + (i % 4) as f32 * 400.0,
                        4700.0 + (i / 4) as f32 * 300.0,
                    );
                    scene3d::draw_ship_mesh(
                        &mut m3,
                        m,
                        pos,
                        180.0,
                        (0.0, 0.0, 0.0),
                        (0.0, 0.0, 0.0),
                        0.7,
                    );
                }
            } else {
                for (pos, heading, _) in ships {
                    scene3d::draw_ship_mesh(
                        &mut m3,
                        model,
                        pos,
                        heading,
                        (0.0, 0.0, 0.0),
                        (0.0, 0.0, 0.0),
                        0.7,
                    );
                }
            }
        }
        for (pos, _, label) in ships.iter().filter(|_| pairs.len() == 1).copied() {
            let s = cam2d.to_screen(pos);
            d.draw_circle(s.0 as i32, s.1 as i32, 4.0, Color::YELLOW);
            d.draw_text(label, s.0 as i32 + 6, s.1 as i32 - 20, 20, Color::YELLOW);
        }
        d.draw_text(
            "red = +x (east), blue = +map y (south); yellow = 2D to_screen",
            10,
            10,
            20,
            Color::WHITE,
        );
        if frame == 2 {
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
