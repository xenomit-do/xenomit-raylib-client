# Mobile controls

The client uses the same touch HUD on Android and in the desktop preview. On Android the HUD starts enabled. On desktop, start with `--mobile-preview` or open the Settings window and enable **Touch controls / mobile preview**. The preview flag selects the layout for that run; the Settings checkbox is saved with the UI preferences.

In landscape, hold and drag the left stick to steer; releasing it stops sending movement orders. The right controls are target cycle, fire, rocket, jump/repair, and P.E.T. Target selects the next NPC. Fire starts laser fire on the selected target or aborts an active laser link. Rocket attacks the selected target. Jump uses a gate, or selects the available repair while the kill screen is open. P.E.T. toggles the pet. The radar opens or closes the map window. A second finger can use an action while the movement stick remains held.

The small arrow in the upper-left collapses or restores the main toolbar. Tap outside the touch controls to use the normal windows and game UI. Modal settings and the kill screen consume touches so they cannot also issue map orders. The standard equipped slot bar remains available; the desktop action bar and slot editor are hidden in touch mode to leave the center clear.

Open the **Settings** toolbar window, then **Render modes**, to choose 2D/3D rendering by component. Changes remain a draft until **Save**; **Cancel** discards them and **Reset** restores the defaults. The **General** button returns to the Settings window. These preferences are stored in `ui_settings.json` beside the client assets.

3D modes use the assets and recipes supplied for each game object. The map-background pass draws authored planes and meshes; authored tilemap layers still use the 2D renderer. Other map display features such as particles, lens flares, and animation nodes are not included in that pass. While 3D assets are loading, the existing 2D backdrop stays visible. A component without a usable 3D recipe continues to use its 2D rendering path.

For sign-in and Android build details, see [BUILDING.md](BUILDING.md).
