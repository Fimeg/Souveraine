//! Serve loop for souveraine-machined.
//!
//! Deliberately synchronous, std-only: one small thread per connection, a
//! few requests per client, everything auditable in one sitting. The daemon
//! is the only process that reads the machine private key; callers get
//! signatures and the public key over the socket, never the key itself.
//!
//! Compiled into the `souveraine-machined` bin via `#[path]` includes, the
//! same pattern as `souveraine-secrets` — `crate::identity` and
//! `crate::protocol` below resolve to the bin's module tree.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::identity::SeedId;
use crate::protocol::{self, Request, MAX_REQUEST_BYTES};

/// SO_PEERCRED identity of the connecting process. Logged on every request
/// so the audit trail names the caller, not just the call.
#[derive(Debug, Clone, Copy)]
struct PeerCred {
    pid: i32,
    uid: u32,
    gid: u32,
}

fn peer_cred(stream: &UnixStream) -> Option<PeerCred> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: SO_PEERCRED fills a ucred struct of the size we pass; the fd
    // is live for the duration of the call because we hold &UnixStream.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    if rc == 0 {
        Some(PeerCred {
            pid: cred.pid,
            uid: cred.uid,
            gid: cred.gid,
        })
    } else {
        None
    }
}

pub fn run(seed: SeedId, socket_path: &Path) -> Result<()> {
    // A live daemon answers on the socket; a stale file from a crash does
    // not. Refuse to double-bind, clean up only what is genuinely dead.
    if socket_path.exists() {
        if UnixStream::connect(socket_path).is_ok() {
            anyhow::bail!(
                "another souveraine-machined is already serving {}",
                socket_path.display()
            );
        }
        warn!("removing stale socket at {}", socket_path.display());
        std::fs::remove_file(socket_path)
            .with_context(|| format!("removing stale socket {}", socket_path.display()))?;
    }

    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("binding {}", socket_path.display()))?;
    // Group-rw: the RuntimeDirectory's 0750 + this 0660 make membership in
    // the `souveraine` group the access control. No auth theater on top —
    // the socket permission IS the policy, and every request is logged.
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o660))
        .with_context(|| format!("setting socket permissions on {}", socket_path.display()))?;

    info!(
        "machine identity {} ({}) serving on {}",
        seed.glyph(),
        seed.public_key_hex(),
        socket_path.display()
    );

    // Signing is &self; the key never needs to be copied, only shared.
    let seed = Arc::new(seed);
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                warn!("accept failed: {e}");
                continue;
            }
        };
        let seed = Arc::clone(&seed);
        std::thread::spawn(move || handle_connection(stream, seed));
    }
    Ok(())
}

fn handle_connection(stream: UnixStream, seed: Arc<SeedId>) {
    let cred = peer_cred(&stream);
    let (uid, gid, pid) = match cred {
        Some(c) => (c.uid, c.gid, c.pid),
        None => {
            warn!("connection without readable peer credentials — refusing");
            return;
        }
    };

    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(10)));

    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(e) => {
            warn!("could not clone stream for uid={uid} pid={pid}: {e}");
            return;
        }
    };
    let mut reader = BufReader::new(stream);

    loop {
        let mut line = String::new();
        // take() caps a single request; a client shoveling an oversized line
        // gets a refusal and the connection dropped, not an OOM.
        match (&mut reader).take(MAX_REQUEST_BYTES).read_line(&mut line) {
            Ok(0) => return, // clean EOF
            Ok(_) => {}
            Err(e) => {
                warn!("read error from uid={uid} pid={pid}: {e}");
                return;
            }
        }
        if line.len() as u64 >= MAX_REQUEST_BYTES {
            let _ = respond(&mut writer, refusal("request exceeds size limit"));
            warn!("oversized request from uid={uid} pid={pid} — connection dropped");
            return;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Request>(line) {
            Ok(req) => handle_request(&seed, req, uid, gid, pid),
            Err(e) => {
                warn!("malformed request from uid={uid} pid={pid}: {e}");
                refusal(&format!("malformed request: {e}"))
            }
        };
        if respond(&mut writer, response).is_err() {
            return;
        }
    }
}

fn handle_request(seed: &SeedId, req: Request, uid: u32, gid: u32, pid: i32) -> serde_json::Value {
    match req {
        Request::Status => serde_json::json!({
            "ok": true,
            "public_key": seed.public_key_hex(),
            "glyph": seed.glyph(),
            "signing_context": protocol::SIGNING_CONTEXT,
        }),
        Request::Pubkey => serde_json::json!({
            "ok": true,
            "public_key": seed.public_key_hex(),
            "glyph": seed.glyph(),
        }),
        Request::Sign {
            domain,
            payload_hex,
        } => {
            if !protocol::valid_domain(&domain) {
                warn!("sign refused for uid={uid} pid={pid}: invalid domain {domain:?}");
                return refusal("invalid domain: ascii alphanumeric/-/_/., max 64 chars");
            }
            let payload = match hex::decode(&payload_hex) {
                Ok(p) => p,
                Err(e) => {
                    warn!("sign refused for uid={uid} pid={pid} domain={domain}: bad payload hex: {e}");
                    return refusal("payload_hex is not valid hex");
                }
            };
            let bytes = protocol::signing_bytes(&domain, &payload);
            let signature = hex::encode(seed.sign(&bytes).to_bytes());
            info!(
                "signed domain={domain} payload_len={} for uid={uid} gid={gid} pid={pid}",
                payload.len()
            );
            serde_json::json!({
                "ok": true,
                "signature": signature,
                "public_key": seed.public_key_hex(),
                "domain": domain,
            })
        }
    }
}

fn refusal(reason: &str) -> serde_json::Value {
    serde_json::json!({ "ok": false, "reason": reason })
}

fn respond(writer: &mut UnixStream, value: serde_json::Value) -> std::io::Result<()> {
    writer.write_all(value.to_string().as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}
