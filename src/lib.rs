//! xenomit-raylib-client: native DarkOrbit client.
//!
//! Reimplementation of the Flash client (`game_resources/main.swf`) on raylib.
//! Speaks the real wire protocol through `xenomit-packet-client` and resolves
//! session/authentication through `xenomit-backpage`, mirroring the stock
//! client's bootstrap:
//!
//! 1. `dosid` cookie -> backpage flashvars (`userID`, `sessionID`, `mapID`,
//!    `cdn`, `basePath`, `dynamicHost`, `resourcesXmlHash`, ...)
//! 2. `{dynamicHost}{basePath}/xml/maps.php` -> map server endpoint
//! 3. `{cdn}{basePath}/xml/resources.xml` -> resource file index
//! 4. embedded XML in main.swf (`game.xml`, `lib_*.xml`) -> patterns/assets
//! 5. TCP connect -> greeting -> version -> login -> ready -> world traffic

pub mod app;
pub mod assets;
pub mod audio;
pub mod client;
pub mod game;
pub mod locale;
pub mod net;
pub mod render;
pub mod swf_extract;
pub mod ui;
