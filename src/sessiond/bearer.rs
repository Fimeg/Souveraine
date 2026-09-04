//! Bearer posture — which link carries what.
//!
//! This is the device-state half of TASK-49. It exists because nothing owned
//! the question: which link carries traffic was the residue of a DHCP metric, a
//! NetworkManager penalty and a shell script's assumption, and when those
//! disagreed the device silently lost the network.
//!
//! Three rules shape everything here, and each was bought with an outage.
//!
//! **The machine reports and decides; it never actuates in the reader.** The
//! previous answer, the old dispatcher gate, was driven by the NM dispatcher *and* a
//! 90 s timer while calling `nmcli connection up/down` — which is itself an NM
//! event. It recycled the tunnel 652 times in 90 minutes, through doze tiers
//! whose contract is "network fetchers stopped", and the user could not switch
//! it off. An actuator wired back into its own sensor. Here the probe is pure
//! reading, the decision is a pure function, and the only way out is an
//! `Action` from `tick()` — DEVICE-STATE-MACHINE §12.
//!
//! **"Associated" is not "carrying."** §10 learned this about sensors: a source
//! that reported and then went silent is `Down`, not quiet. A wifi link that
//! associates and cannot route is exactly a sensor that heartbeats and lies,
//! and it is what NetworkManager's own connectivity penalty mislabels — +20000
//! on the link, so the degraded cellular default wins instead. `LinkHealth`
//! keeps the two apart so a link that lies cannot win.
//!
//! **"Am I home" is not an address-prefix question.** The old gate matched a
//! /16 address prefix on `wlan0`. Home was a /24 inside it, and this phone
//! spends its time on a foreign network in a neighbouring /24 that the test
//! therefore read as home, taking the tunnel down as "redundant". Identity,
//! never prefix: the SSID, or a host only the home LAN serves.
//!
//! What this module deliberately does **not** decide: whether the tunnel is up.
//! That is the user's switch and stays the user's switch. This decides how
//! traffic is carried, which is a different question and must not quietly
//! re-acquire the first.

use std::process::Command;

use serde::{Deserialize, Serialize};

/// A physical carrier of traffic. The tunnel is not one of these — it rides
/// whichever of these is chosen, which is the whole point of pinning it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bearer {
    Wifi,
    Cellular,
}

impl Bearer {
    pub fn as_str(self) -> &'static str {
        match self {
            Bearer::Wifi => "wifi",
            Bearer::Cellular => "cellular",
        }
    }
}

/// How much a link is actually doing — not what it claims.
///
/// The ordering is deliberate and is the comparison the decision uses:
/// `Carrying` outranks `Associated` outranks `Down` outranks `Absent`. A link
/// may only win if something has been shown to cross it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkHealth {
    /// No such link on this device right now.
    Absent,
    /// The link exists and is known not to be usable.
    Down,
    /// Associated and addressed, but nothing has been shown to cross it. This
    /// is the honest default for a link we have not probed — it is *not* a
    /// claim that the link works.
    Associated,
    /// Associated and carrying: a default route exists over it and the link is
    /// not under a connectivity penalty.
    Carrying,
}

impl LinkHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkHealth::Absent => "absent",
            LinkHealth::Down => "down",
            LinkHealth::Associated => "associated",
            LinkHealth::Carrying => "carrying",
        }
    }

    /// May this link be preferred at all? `Associated` is allowed to win when
    /// nothing better exists, but `Down`/`Absent` never are.
    fn usable(self) -> bool {
        matches!(self, LinkHealth::Associated | LinkHealth::Carrying)
    }
}

/// The tunnel's own health, which is a different question from a link's.
///
/// It can be `up` in NetworkManager and deaf on the wire — that is exactly what
/// was measured over clat on 2026-07-31: `368 B received, 5.30 KiB sent`, a
/// handshake going stale against a 25 s keepalive, while every status readout
/// said connected. `wg`'s received-byte counter is the honest instrument, the
/// way logind is for the lock, and until this module nothing read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelHealth {
    /// The toggle is off. The user's authority, not ours — never an error.
    Off,
    /// Up and the far end is answering.
    Handshaking,
    /// Up, sending, and nothing is coming back. Reported rather than left
    /// looking connected.
    Deaf,
}

impl TunnelHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            TunnelHealth::Off => "off",
            TunnelHealth::Handshaking => "handshaking",
            TunnelHealth::Deaf => "deaf",
        }
    }
}

/// What the machine believes about how traffic can be carried right now.
///
/// Every field is evidence with a source, not a fact — §9. `home` is `Option`
/// precisely because "I do not know where I am" is a third answer, and the old
/// gate's bug was collapsing it into `false`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BearerEvidence {
    pub wifi: LinkHealth,
    pub cellular: LinkHealth,
    pub tunnel: TunnelHealth,
    /// `Some(true)` on the home LAN, `Some(false)` demonstrably elsewhere,
    /// `None` when we cannot tell. Derived from SSID identity, never from an
    /// address prefix.
    pub home: Option<bool>,
    /// The SSID we are associated with, when there is one. Kept for the
    /// readout so a human can see *why* `home` reads as it does.
    pub ssid: Option<String>,
}

impl Default for BearerEvidence {
    fn default() -> Self {
        Self {
            wifi: LinkHealth::Absent,
            cellular: LinkHealth::Absent,
            tunnel: TunnelHealth::Off,
            home: None,
            ssid: None,
        }
    }
}

impl BearerEvidence {
    /// Which bearer ordinary traffic should prefer, if any.
    ///
    /// Pure function of the evidence — no clock, no I/O, no memory. That is
    /// what makes the flapping question answerable elsewhere and this testable
    /// here.
    ///
    /// Wifi wins ties because it is the unmetered link, but only on equal
    /// health: a *carrying* cellular link beats a merely *associated* wifi one,
    /// which is the case NetworkManager's penalty gets backwards.
    pub fn preferred(&self) -> Option<Bearer> {
        match (self.wifi.usable(), self.cellular.usable()) {
            (false, false) => None,
            (true, false) => Some(Bearer::Wifi),
            (false, true) => Some(Bearer::Cellular),
            (true, true) => {
                if self.cellular > self.wifi {
                    Some(Bearer::Cellular)
                } else {
                    Some(Bearer::Wifi)
                }
            }
        }
    }

    /// Which bearer the tunnel's endpoint should be pinned to.
    ///
    /// The same link ordinary traffic prefers, and `None` when the tunnel is
    /// off — pinning an endpoint for a tunnel the user has switched off would
    /// be this module deciding something that is not its to decide.
    pub fn tunnel_underlay(&self) -> Option<Bearer> {
        match self.tunnel {
            TunnelHealth::Off => None,
            TunnelHealth::Handshaking | TunnelHealth::Deaf => self.preferred(),
        }
    }

    /// Is the tunnel up but not being answered? Worth surfacing on its own
    /// because every other readout on the device calls this "connected".
    pub fn tunnel_is_deaf(&self) -> bool {
        matches!(self.tunnel, TunnelHealth::Deaf)
    }
}

/// Read the current bearer posture off the system.
///
/// Reading, and only reading — the same contract `lockhint` has with logind.
/// Anything that changes state leaves as an `Action`.
pub fn probe(home_ssids: &[String]) -> BearerEvidence {
    let (wifi, ssid) = probe_wifi(home_ssids);
    BearerEvidence {
        wifi: wifi.0,
        cellular: probe_cellular(),
        tunnel: probe_tunnel(),
        home: wifi.1,
        ssid,
    }
}

/// `nmcli -t -f DEVICE,TYPE,STATE,CONNECTION device` is the one call that
/// answers association without asking NetworkManager for its opinion about
/// connectivity — which is the opinion that mislabels the link.
fn nmcli_devices() -> Vec<(String, String, String, String)> {
    let out = match Command::new("nmcli")
        .args(["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|l| {
            let mut f = l.splitn(4, ':');
            Some((
                f.next()?.to_string(),
                f.next()?.to_string(),
                f.next()?.to_string(),
                f.next().unwrap_or("").to_string(),
            ))
        })
        .collect()
}

/// Does a default route exist over this interface? This is the "carrying" half
/// of the distinction — association alone is not evidence of a path.
fn has_default_route(iface: &str) -> bool {
    let out = match Command::new("ip")
        .args(["-4", "route", "show", "default"])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return false,
    };
    String::from_utf8_lossy(&out)
        .lines()
        .any(|l| l.split_whitespace().any(|w| w == iface))
}

/// Returns wifi health, whether we are home, and the SSID.
fn probe_wifi(home_ssids: &[String]) -> ((LinkHealth, Option<bool>), Option<String>) {
    let devices = nmcli_devices();
    let Some((iface, _, state, _)) = devices.iter().find(|(_, t, _, _)| t == "wifi").cloned()
    else {
        return ((LinkHealth::Absent, None), None);
    };

    if state != "connected" {
        return ((LinkHealth::Down, Some(false)), None);
    }

    let ssid = active_ssid(&iface);
    // Identity, never prefix. An unknown SSID is `Some(false)` — we know where
    // we are and it is not home. No SSID at all is `None` — we do not know.
    let home = ssid.as_ref().map(|s| home_ssids.iter().any(|h| h == s));

    let health = if has_default_route(&iface) {
        LinkHealth::Carrying
    } else {
        LinkHealth::Associated
    };
    ((health, home), ssid)
}

fn active_ssid(iface: &str) -> Option<String> {
    let out = Command::new("nmcli")
        .args([
            "-t",
            "-f",
            "ACTIVE,SSID",
            "device",
            "wifi",
            "list",
            "ifname",
            iface,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("yes:"))
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

fn probe_cellular() -> LinkHealth {
    let devices = nmcli_devices();
    // clat is the v4-over-v6 shim the carrier path rides; on this device the
    // modem itself shows up as a `gsm` device and clat as a separate tun.
    let modem_up = devices
        .iter()
        .any(|(_, t, s, _)| (t == "gsm" || t == "cdma") && s == "connected");
    if !modem_up {
        return LinkHealth::Absent;
    }
    if has_default_route("clat")
        || devices
            .iter()
            .any(|(d, _, s, _)| d == "clat" && s == "connected")
    {
        LinkHealth::Carrying
    } else {
        LinkHealth::Associated
    }
}

/// Received bytes are the tunnel's honest instrument: a peer that has sent and
/// never received is up and deaf, not connected.
///
/// Read from sysfs, not from `wg show`. `wg show` needs `CAP_NET_ADMIN` for
/// `WG_CMD_GET_DEVICE`, and sessiond is a systemd *user* unit — measured on the
/// phone 2026-08-01, it gets "Unable to access interface `wg0`: Operation not
/// permitted" and every tunnel therefore read `off` while one was up and
/// handshaking. `/sys/class/net/*/statistics/rx_bytes` is world-readable and is
/// the same number, without the parse or the privilege.
fn probe_tunnel() -> TunnelHealth {
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return TunnelHealth::Off;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // DEVTYPE is how the kernel names the link's family; NM tears the
        // interface down entirely when the profile goes down, so a wireguard
        // DEVTYPE present at all means a tunnel is configured and up.
        let is_wg = std::fs::read_to_string(path.join("uevent"))
            .map(|s| s.lines().any(|l| l == "DEVTYPE=wireguard"))
            .unwrap_or(false);
        if !is_wg {
            continue;
        }
        let rx = read_counter(&path.join("statistics/rx_bytes"));
        // Nonzero received is the only positive evidence there is. Zero covers
        // both "up and ignored" and the brief window after activation before
        // anything has been sent; both are honestly "not answered yet", and
        // the settling window is longer than that window, so a transient
        // cannot produce an action on its own.
        return if rx > 0 {
            TunnelHealth::Handshaking
        } else {
            TunnelHealth::Deaf
        };
    }
    TunnelHealth::Off
}

fn read_counter(path: &std::path::Path) -> u64 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(wifi: LinkHealth, cellular: LinkHealth) -> BearerEvidence {
        BearerEvidence {
            wifi,
            cellular,
            ..Default::default()
        }
    }

    #[test]
    fn nothing_usable_prefers_nothing() {
        assert_eq!(ev(LinkHealth::Absent, LinkHealth::Absent).preferred(), None);
        assert_eq!(ev(LinkHealth::Down, LinkHealth::Down).preferred(), None);
    }

    #[test]
    fn a_carrying_link_beats_a_merely_associated_one() {
        // This is the case NetworkManager gets backwards: it penalises the
        // associated-but-unvalidated wifi link by +20000 and lets the degraded
        // clat default win. Here the *carrying* link wins on its own merit.
        assert_eq!(
            ev(LinkHealth::Associated, LinkHealth::Carrying).preferred(),
            Some(Bearer::Cellular)
        );
        assert_eq!(
            ev(LinkHealth::Carrying, LinkHealth::Associated).preferred(),
            Some(Bearer::Wifi)
        );
    }

    #[test]
    fn wifi_wins_only_on_equal_health() {
        assert_eq!(
            ev(LinkHealth::Carrying, LinkHealth::Carrying).preferred(),
            Some(Bearer::Wifi)
        );
        assert_eq!(
            ev(LinkHealth::Associated, LinkHealth::Associated).preferred(),
            Some(Bearer::Wifi)
        );
    }

    #[test]
    fn a_down_link_never_wins_even_alone() {
        assert_eq!(
            ev(LinkHealth::Down, LinkHealth::Absent).preferred(),
            None,
            "a link known not to work is not a fallback"
        );
        assert_eq!(
            ev(LinkHealth::Down, LinkHealth::Associated).preferred(),
            Some(Bearer::Cellular)
        );
    }

    #[test]
    fn tunnel_off_pins_nothing() {
        let e = BearerEvidence {
            wifi: LinkHealth::Carrying,
            tunnel: TunnelHealth::Off,
            ..Default::default()
        };
        assert_eq!(
            e.tunnel_underlay(),
            None,
            "the toggle is the user's; a tunnel that is off gets no underlay"
        );
    }

    #[test]
    fn a_deaf_tunnel_still_gets_pinned_and_is_reported() {
        // Deaf is the case worth acting on: it is up, so the user asked for it,
        // and it is not being answered, so the underlay is the suspect.
        let e = BearerEvidence {
            wifi: LinkHealth::Carrying,
            tunnel: TunnelHealth::Deaf,
            ..Default::default()
        };
        assert_eq!(e.tunnel_underlay(), Some(Bearer::Wifi));
        assert!(e.tunnel_is_deaf());
    }

    #[test]
    fn home_is_identity_not_prefix() {
        // The regression this whole module exists to not repeat: a foreign
        // network in a neighbouring /24 that a /16 prefix test reads as home.
        let home = vec!["home-ssid".to_string()];
        assert_eq!(
            home.iter().any(|h| h == "some-cafe"),
            false,
            "a foreign SSID is not home however its addresses look"
        );
        assert!(home.iter().any(|h| h == "home-ssid"));
    }
}
