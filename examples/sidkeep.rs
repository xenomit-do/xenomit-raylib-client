//! Dev tool: keep a `dosid` session warm across test runs.
//!
//!   cargo run --release --example sidkeep -- --server tr3 --dosid X [--every-secs 240]
//!
//! Warms the session once with `launch::load`, then probes `check_sid`
//! on a fixed cadence and prints one timestamped status line per probe.
//!
//! Shop safety: `check_sid` only takes the `shop_probe` path — which
//! *buys ammo* — when `last_shop_request` is armed by a `post*` call to
//! `ajax/shop.php`. Nothing on this path ever arms it: `launch::load`
//! does one `get` plus a raw `send` to `flashAPI/loadingScreen.php`
//! (neither goes through `post_wait`), and `check_sid` itself only
//! issues `get_wait`. The shop probe can therefore never fire here.

use anyhow::{Context, Result};
use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use xenomit_backpage::client::SidStatus;

fn main() -> Result<()> {
    let mut server = "tr3".to_owned();
    let mut dosid = String::new();
    let mut every_secs = 240u64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--server" => server = args.next().context("--server needs a value")?,
            "--dosid" => dosid = args.next().context("--dosid needs a value")?,
            "--every-secs" => {
                every_secs = args.next().context("--every-secs needs a value")?.parse()?
            }
            _ => anyhow::bail!("unknown arg {a}"),
        }
    }
    if dosid.is_empty() {
        anyhow::bail!("usage: sidkeep --server tr3 --dosid X [--every-secs 240]");
    }

    let session = xenomit_backpage::session::Session::new(&dosid, &server);
    let mut client = xenomit_backpage::client::BackpageClient::new(session)
        .context("could not create backpage client")?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(xenomit_backpage::launch::load(&mut client))
        .context("backpage launch failed (expired dosid?)")?;
    println!(
        "{} launch ok — sid on {}",
        ts(),
        client.session().instance()
    );
    let _ = std::io::stdout().flush();

    loop {
        // 30s ceiling: `check_sid` has no request timeout of its own and
        // a stalled probe once hung this loop for hours. A timeout is a
        // status line, not a crash — the next tick tries again.
        let status = rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(30), client.check_sid()).await
        });
        match status {
            Ok(status) => {
                println!("{} sid {:?} ({status})", ts(), status);
                let _ = std::io::stdout().flush();
                if status == SidStatus::Invalid {
                    eprintln!("{} sid expired — stopping", ts());
                    std::process::exit(2);
                }
            }
            Err(_) => {
                println!("{} sid check timed out after 30s", ts());
                let _ = std::io::stdout().flush();
            }
        }
        std::thread::sleep(Duration::from_secs(every_secs));
    }
}

/// Seconds since epoch — good enough for a keepalive log line.
fn ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
