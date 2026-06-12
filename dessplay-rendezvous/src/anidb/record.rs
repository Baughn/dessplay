//! Record/replay support for real AniDB exchanges.
//!
//! `anidb-probe scan` wraps its UDP socket in a [`RecordingWire`] that
//! writes every completed query→response pair to a testdata file,
//! **sanitized at write time**: credentials and session keys never
//! reach disk. The replay test (`tests/anidb_replay.rs`) then runs the
//! real codec over those recordings — fixtures captured from the live
//! server, exercised forever offline.

use std::io::Write;
use std::sync::Mutex;

use super::client::Wire;

/// Replacement for redacted usernames.
const USER_REDACTED: &str = "USER";
/// Replacement for redacted passwords.
const PASS_REDACTED: &str = "REDACTED";
/// Replacement for redacted session keys (alphanumeric, so
/// [`super::protocol::session_key`] still parses it).
const SESSION_REDACTED: &str = "SESSION";

/// Redact credentials and session keys from an outgoing command.
pub fn sanitize_request(request: &str) -> String {
    let Some((command, params)) = request.split_once(' ') else {
        return request.to_string();
    };
    let params = params
        .split('&')
        .map(|kv| match kv.split_once('=') {
            Some(("user", _)) => format!("user={USER_REDACTED}"),
            Some(("pass", _)) => format!("pass={PASS_REDACTED}"),
            Some(("s", _)) => format!("s={SESSION_REDACTED}"),
            _ => kv.to_string(),
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{command} {params}")
}

/// Redact the session key from an AUTH reply (`[tag ]200 KEY LOGIN
/// ACCEPTED`). Other responses pass through unchanged.
pub fn sanitize_response(response: &str) -> String {
    let mut lines = response.lines();
    let Some(header) = lines.next() else {
        return response.to_string();
    };
    let mut tokens: Vec<&str> = header.split(' ').collect();
    let code_index = tokens
        .iter()
        .position(|t| t.len() == 3 && t.bytes().all(|b| b.is_ascii_digit()));
    if let Some(index) = code_index
        && (tokens[index] == "200" || tokens[index] == "201")
        && index + 1 < tokens.len()
    {
        tokens[index + 1] = SESSION_REDACTED;
    }
    let mut sanitized = tokens.join(" ");
    for line in lines {
        sanitized.push('\n');
        sanitized.push_str(line);
    }
    sanitized
}

/// One recorded exchange in the testdata format:
///
/// ```text
/// >>> FILE size=1&ed2k=ab..&fmask=..&amask=..&s=SESSION&tag=t2
/// <<< t2 220 FILE
/// <<< 312498|8692|Sousou no Frieren|Frieren|01
///
/// ```
/// (pairs separated by a blank line)
pub fn write_exchange(
    out: &mut impl Write,
    request: &str,
    response: &str,
) -> std::io::Result<()> {
    writeln!(out, ">>> {request}")?;
    for line in response.lines() {
        writeln!(out, "<<< {line}")?;
    }
    writeln!(out)
}

/// Parse a testdata file back into (request, response) pairs. Lines
/// outside the `>>>`/`<<<` markers (comments, blanks) are separators.
pub fn parse_exchanges(text: &str) -> Vec<(String, String)> {
    let mut exchanges = Vec::new();
    let mut request: Option<String> = None;
    let mut response = String::new();
    let mut flush = |request: &mut Option<String>, response: &mut String| {
        if let Some(req) = request.take() {
            if !response.is_empty() {
                exchanges.push((req, std::mem::take(response)));
            } else {
                response.clear();
            }
        }
        response.clear();
    };
    for line in text.lines() {
        if let Some(req) = line.strip_prefix(">>> ") {
            flush(&mut request, &mut response);
            request = Some(req.to_string());
        } else if let Some(part) = line.strip_prefix("<<< ") {
            if !response.is_empty() {
                response.push('\n');
            }
            response.push_str(part);
        } else {
            flush(&mut request, &mut response);
        }
    }
    flush(&mut request, &mut response);
    exchanges
}

struct RecordState<O> {
    /// The last sent (sanitized) request, awaiting its reply.
    pending: Option<String>,
    out: O,
    exchanges: usize,
}

/// A [`Wire`] wrapper that records completed exchanges. Requests that
/// never get a reply (timeouts) are silently dropped — only pairs are
/// useful as parser fixtures.
pub struct RecordingWire<W, O> {
    inner: W,
    state: Mutex<RecordState<O>>,
}

impl<W: Wire, O: Write + Send + 'static> RecordingWire<W, O> {
    /// Wrap `inner`, appending recorded exchanges to `out`.
    pub fn new(inner: W, out: O) -> Self {
        Self {
            inner,
            state: Mutex::new(RecordState {
                pending: None,
                out,
                exchanges: 0,
            }),
        }
    }

    /// How many exchanges were recorded so far.
    pub fn exchanges(&self) -> usize {
        match self.state.lock() {
            Ok(state) => state.exchanges,
            Err(poisoned) => poisoned.into_inner().exchanges,
        }
    }
}

impl<W: Wire, O: Write + Send + 'static> Wire for RecordingWire<W, O> {
    async fn send(&self, payload: &str) -> std::io::Result<()> {
        self.inner.send(payload).await?;
        if let Ok(mut state) = self.state.lock() {
            state.pending = Some(sanitize_request(payload));
        }
        Ok(())
    }

    async fn recv(&self) -> std::io::Result<String> {
        let raw = self.inner.recv().await?;
        if let Ok(mut state) = self.state.lock()
            && let Some(request) = state.pending.take()
        {
            let response = sanitize_response(&raw);
            let result = write_exchange(&mut state.out, &request, &response)
                .and_then(|()| state.out.flush());
            match result {
                Ok(()) => state.exchanges += 1,
                Err(e) => tracing::error!("recording exchange failed: {e}"),
            }
        }
        Ok(raw)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn requests_lose_credentials_and_sessions() {
        let auth = sanitize_request(
            "AUTH user=baughn&pass=hunter2&protover=3&client=dessplay&clientver=1&enc=UTF8&tag=t1",
        );
        assert_eq!(
            auth,
            "AUTH user=USER&pass=REDACTED&protover=3&client=dessplay&clientver=1&enc=UTF8&tag=t1"
        );
        let file = sanitize_request("FILE size=1&ed2k=ab&fmask=40&amask=00&s=xK3fp&tag=t2");
        assert_eq!(file, "FILE size=1&ed2k=ab&fmask=40&amask=00&s=SESSION&tag=t2");
    }

    #[test]
    fn auth_replies_lose_the_session_key() {
        assert_eq!(
            sanitize_response("t1 200 xK3fp LOGIN ACCEPTED"),
            "t1 200 SESSION LOGIN ACCEPTED"
        );
        assert_eq!(
            sanitize_response("201 xK3fp LOGIN ACCEPTED - NEW VERSION AVAILABLE"),
            "201 SESSION LOGIN ACCEPTED - NEW VERSION AVAILABLE"
        );
        // Non-auth responses (and data lines) are untouched.
        let file = "t2 220 FILE\n1|2|Romaji|English|01";
        assert_eq!(sanitize_response(file), file);
    }

    #[test]
    fn exchanges_round_trip_through_the_format() {
        let mut out = Vec::new();
        write_exchange(&mut out, "PING tag=t1", "t1 300 PONG").unwrap();
        write_exchange(
            &mut out,
            "FILE size=1&s=SESSION&tag=t2",
            "t2 220 FILE\n1|2|A|B|01",
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        let parsed = parse_exchanges(&text);
        assert_eq!(
            parsed,
            vec![
                ("PING tag=t1".to_string(), "t1 300 PONG".to_string()),
                (
                    "FILE size=1&s=SESSION&tag=t2".to_string(),
                    "t2 220 FILE\n1|2|A|B|01".to_string()
                ),
            ]
        );
    }

    #[test]
    fn parser_skips_comments_and_orphans() {
        let text = "# a comment\n>>> orphaned request\n\n>>> PING tag=t1\n<<< t1 300 PONG\n";
        assert_eq!(
            parse_exchanges(text),
            vec![("PING tag=t1".to_string(), "t1 300 PONG".to_string())]
        );
    }

    #[tokio::test]
    async fn recording_wire_pairs_and_sanitizes() {
        use std::sync::Arc;

        /// A trivial echo wire for the test.
        struct EchoWire(Mutex<Vec<String>>);
        impl Wire for Arc<EchoWire> {
            async fn send(&self, payload: &str) -> std::io::Result<()> {
                self.0.lock().unwrap().push(payload.to_string());
                Ok(())
            }
            async fn recv(&self) -> std::io::Result<String> {
                Ok("t1 200 xK3fp LOGIN ACCEPTED".to_string())
            }
        }

        let shared: Arc<Mutex<Vec<u8>>> = Arc::default();
        /// Write adapter over the shared buffer.
        struct SharedOut(Arc<Mutex<Vec<u8>>>);
        impl Write for SharedOut {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let wire = RecordingWire::new(
            Arc::new(EchoWire(Mutex::new(Vec::new()))),
            SharedOut(Arc::clone(&shared)),
        );
        wire.send("AUTH user=u&pass=p&tag=t1").await.unwrap();
        let raw = wire.recv().await.unwrap();
        // The caller still sees the real session key...
        assert!(raw.contains("xK3fp"));
        assert_eq!(wire.exchanges(), 1);
        // ...but the recording has neither credentials nor key.
        let recorded = String::from_utf8(shared.lock().unwrap().clone()).unwrap();
        assert!(!recorded.contains("pass=p"));
        assert!(!recorded.contains("xK3fp"));
        let pairs = parse_exchanges(&recorded);
        assert_eq!(pairs[0].0, "AUTH user=USER&pass=REDACTED&tag=t1");
        assert_eq!(pairs[0].1, "t1 200 SESSION LOGIN ACCEPTED");
    }
}
