use xenomit_client::assets::uberlib::TextureQuality;
use xenomit_client::game::{
    settings_ui::{component_mode_buttons, quality_mode_buttons, render_layout, RenderSetting},
    Game,
};
use xenomit_client::ui::preferences::UiPreferences;

fn press(game: &mut Game, action: RenderSetting) {
    let (vw, vh) = game.camera.viewport;
    let rect = render_layout(vw, vh)
        .1
        .into_iter()
        .find(|(item, _)| *item == action)
        .unwrap()
        .1;
    // Component rows only react to their 2D/3D mode buttons — press the 3D
    // button so the click follows the same route a real pointer would.
    let p = if RenderSetting::COMPONENTS.contains(&action) {
        let (_, three_d) = component_mode_buttons(rect);
        (three_d.0 + three_d.2 * 0.5, three_d.1 + three_d.3 * 0.5)
    } else {
        (rect.0 + rect.2 * 0.5, rect.1 + rect.3 * 0.5)
    };
    assert!(game.handle_render_setting_click(p));
}

fn press_quality(game: &mut Game, quality: TextureQuality) {
    let (vw, vh) = game.camera.viewport;
    let row = render_layout(vw, vh)
        .1
        .into_iter()
        .find(|(item, _)| *item == RenderSetting::TextureQuality)
        .unwrap()
        .1;
    let i = TextureQuality::ALL
        .iter()
        .position(|q| *q == quality)
        .unwrap();
    let b = quality_mode_buttons(row)[i];
    assert!(game.handle_render_setting_click((b.0 + b.2 * 0.5, b.1 + b.3 * 0.5)));
}

#[test]
fn component_settings_cancel_and_save_follow_the_actual_pointer_route() {
    let mut game = Game::new(1, (21000.0, 13100.0));
    game.camera.viewport = (1280.0, 576.0);
    game.open_render_settings();
    press(&mut game, RenderSetting::Ships);
    press_quality(&mut game, TextureQuality::Low);
    assert!(
        !game.preferences.render.ships_3d,
        "draft changes must not commit before Save"
    );
    assert_eq!(
        game.preferences.render.texture_quality,
        TextureQuality::High,
        "quality is draft-only until Save"
    );
    assert!(
        game.handle_render_setting_click((0.0, 0.0)),
        "modal backdrop must consume world clicks"
    );
    press(&mut game, RenderSetting::Cancel);
    assert!(!game.show_render_settings);
    assert!(!game.preferences.render.ships_3d);
    assert_eq!(
        game.preferences.render.texture_quality,
        TextureQuality::High
    );

    let directory =
        std::env::temp_dir().join(format!("xenomit-render-settings-{}", std::process::id()));
    let path = directory.join("ui_settings.json");
    game.preferences.set_path(path.clone());
    game.open_render_settings();
    press(&mut game, RenderSetting::Ships);
    press(&mut game, RenderSetting::Portals);
    press_quality(&mut game, TextureQuality::Medium);
    press(&mut game, RenderSetting::Save);
    assert!(!game.show_render_settings);
    let mut loaded = UiPreferences::default();
    loaded.set_path(path);
    assert!(loaded.render.ships_3d && loaded.render.portals_3d);
    assert!(
        !loaded.render.npcs_3d && !loaded.render.camera_3d,
        "other components stay independent"
    );
    assert_eq!(
        loaded.render.texture_quality,
        TextureQuality::Medium,
        "quality persists with the other render prefs"
    );
    game.open_render_settings();
    press(&mut game, RenderSetting::Reset);
    press(&mut game, RenderSetting::Cancel);
    assert!(
        game.preferences.render.ships_3d,
        "Reset edits the draft until Save"
    );
    let _ = std::fs::remove_dir_all(directory);
}
