use xenomit_packet_client::modules::id;
use xenomit_packet_client::packet::{Packet, Value};

use crate::net::world::SharedWorld;

/// `KillScreenPostCommand` — hero death notice + repair option list.
pub(crate) const KILL_SCREEN_POST: i16 = 252;
/// `KillScreenOptionModule` — one row inside `options`.
const KILL_SCREEN_OPTION_MODULE: i16 = 253;
/// `KillScreenUpdateCommand` — refreshed repair options (cooldowns etc.).
pub(crate) const KILL_SCREEN_UPDATE: i16 = 6777;

/// One repair option from the kill screen (`KillScreenOptionModule`).
#[derive(Clone, Debug, Default)]
pub struct KillScreenOption {
    /// `repairType.repairTypeValue` — `OPTIONTYPE_*` (1 = basic respawn
    /// at the home base; other ids carry a price/cooldown).
    pub repair_type: i32,
    /// `price.amount` (0 = free).
    pub cost: i64,
    /// `price.type` — `PriceModule.type` (0 credits, 1 uridium).
    pub currency: i32,
    /// `affordableForPlayer`.
    pub affordable: bool,
    /// `cooldownTime` (ms) — option disabled until it ticks down.
    pub cooldown_ms: i64,
    /// `buttonKey.baseKey` — localization key of the option's button.
    pub button_key: String,
}

/// In-session kill screen (`KillScreenPostCommand`/`KillScreenUpdate`).
#[derive(Clone, Debug, Default)]
pub struct KillScreen {
    /// `killerName` from the post command ("" when the row was only an
    /// update or synthesized locally).
    pub killer_name: String,
    /// Repair options offered this death (may be empty when the server
    /// hasn't sent the list yet — basic repair is still implied).
    pub options: Vec<KillScreenOption>,
}

impl KillScreen {
    /// `OPTIONTYPE_BASIC_REPAIR` — respawn at the home base.
    pub const BASIC_REPAIR: i32 = 1;

    /// Which `repairType` to send in `KillScreenRepairRequest.selection`:
    /// the free basic repair when offered and affordable, otherwise the
    /// cheapest affordable option, otherwise basic repair (the implied
    /// default when the option list never arrived — same as the
    /// post-login `status 7` flow).
    pub fn best_repair_type(&self) -> i32 {
        let basic = self
            .options
            .iter()
            .find(|o| o.repair_type == Self::BASIC_REPAIR);
        if let Some(o) = basic {
            if o.affordable && o.cooldown_ms <= 0 {
                return o.repair_type;
            }
        }
        self.options
            .iter()
            .filter(|o| o.affordable && o.cooldown_ms <= 0)
            .min_by_key(|o| o.cost)
            .or(basic)
            .map(|o| o.repair_type)
            .unwrap_or(Self::BASIC_REPAIR)
    }
}

pub fn handle(world: &mut SharedWorld, p: &Packet) {
    match p.id {
        id::SHIP_DESTROYED => {
            if !world.expect_disconnect {
                if let (Some(h), Ok(uid)) = (world.world.hero.as_ref(), p.int("destroyedUserId")) {
                    if h.user_id == uid as i32 && world.kill_screen.is_none() {
                        world.kill_screen = Some(KillScreen::default());
                        world.push_log("ship destroyed".to_owned());
                    }
                }
            }
        }
        KILL_SCREEN_POST => {
            let mut ks = world.kill_screen.take().unwrap_or_default();
            ks.killer_name = p.string("killerName").unwrap_or("").to_owned();
            ks.options = kill_screen_options(p);
            world.kill_screen = Some(ks);
            world.push_log("kill screen".to_owned());
        }
        KILL_SCREEN_UPDATE => {
            let options = kill_screen_options(p);
            world
                .kill_screen
                .get_or_insert_with(KillScreen::default)
                .options = options;
        }
        _ => {}
    }
}

/// Parse the `options` vector of `KillScreenPostCommand`/
/// `KillScreenUpdateCommand` — each element is a `KillScreenOptionModule`.
fn kill_screen_options(p: &Packet) -> Vec<KillScreenOption> {
    p.list("options")
        .unwrap_or(&[])
        .iter()
        .filter_map(|v| {
            let Value::Module(m) = v else {
                return None;
            };
            if m.id != KILL_SCREEN_OPTION_MODULE {
                return None;
            }
            let price = m.module("price").ok();
            Some(KillScreenOption {
                repair_type: m
                    .module("repairType")
                    .ok()
                    .and_then(|t| t.int("repairTypeValue").ok())
                    .unwrap_or(-1) as i32,
                cost: price.and_then(|p| p.int("amount").ok()).unwrap_or(0),
                currency: price.and_then(|p| p.int("type").ok()).unwrap_or(0) as i32,
                affordable: m.boolean("affordableForPlayer").unwrap_or(false),
                cooldown_ms: m.int("cooldownTime").unwrap_or(0),
                button_key: m
                    .module("buttonKey")
                    .ok()
                    .and_then(|b| b.string("baseKey").ok())
                    .unwrap_or("")
                    .to_owned(),
            })
        })
        .collect()
}
