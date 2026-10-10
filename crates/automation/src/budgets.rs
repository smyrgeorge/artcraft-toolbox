//! Fixed ceilings for outbound JSON (PhotoCraft's `budgets.rs`, ported without the image
//! previews). These bound individual replies, not the duration or memory of a command.

use std::io::{self, Write};

use serde::Serialize;
use serde_json::{Value, json};

use crate::AutomationError;
use crate::security::MAX_REQUEST_BYTES;

/// Maximum encoded JSON reply, including its newline for JSON-lines transports.
pub const MAX_RESPONSE_BYTES: usize = 8 << 20;

fn bad(message: impl Into<String>) -> AutomationError {
    AutomationError::BadRequest(message.into())
}

struct LimitedWriter {
    bytes: Vec<u8>,
    maximum: usize,
}

impl Write for LimitedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > self.maximum.saturating_sub(self.bytes.len()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("response exceeds {} bytes", self.maximum)));
        }
        self.bytes.try_reserve(buf.len()).map_err(|error| io::Error::other(format!("response allocation failed: {error}")))?;
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Serialize into a bounded buffer. No partial response is sent to the transport.
pub fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>, AutomationError> {
    encode_with_limit(value, MAX_RESPONSE_BYTES - 1)
}

fn encode_with_limit(value: &impl Serialize, maximum: usize) -> Result<Vec<u8>, AutomationError> {
    let mut writer = LimitedWriter { bytes: Vec::new(), maximum };
    serde_json::to_writer(&mut writer, value).map_err(|error| bad(format!("response encoding failed: {error}")))?;
    Ok(writer.bytes)
}

/// Encode the entire envelope before writing. A rejected reply is replaced by a small
/// error with the original ID; the operation may already have completed.
pub fn write_reply(out: &mut impl Write, reply: &Value) -> io::Result<()> {
    let encoded = match json_bytes(reply) {
        Ok(bytes) => bytes,
        Err(_) => {
            let error = json!({
                "id": reply.get("id").cloned().unwrap_or(Value::Null),
                "ok": false,
                "error": format!("response exceeds {MAX_RESPONSE_BYTES} bytes; operation may have completed"),
            });
            match json_bytes(&error) {
                Ok(bytes) => bytes,
                Err(_) => b"{\"id\":null,\"ok\":false,\"error\":\"response budget exceeded\"}".to_vec(),
            }
        }
    };
    out.write_all(&encoded)?;
    out.write_all(b"\n")
}

/// Reserve envelope/ID space and bound retained batch results incrementally, so
/// many individually valid replies cannot accumulate into an enormous batch.
pub struct BatchReplyBudget {
    pub(crate) remaining: usize,
    /// Charge each result as it costs inside a JSON string (MCP text content).
    pub(crate) escaped: bool,
}

impl Default for BatchReplyBudget {
    fn default() -> Self {
        Self { remaining: MAX_RESPONSE_BYTES - MAX_REQUEST_BYTES - 4096, escaped: false }
    }
}

impl BatchReplyBudget {
    /// For a batch reply sent as MCP text content: the encoded reply is embedded in a JSON
    /// string, which escapes every quote and backslash again.
    pub fn escaped() -> Self {
        Self { escaped: true, ..Self::default() }
    }

    /// Must be called before retaining the result or dispatching the next step.
    pub fn charge(&mut self, result: &Value) -> Result<(), AutomationError> {
        let limit = self.remaining.saturating_sub(1);
        let bytes = encode_with_limit(result, limit)?;
        // Compact JSON has no raw control characters, so only `"` and `\` grow when escaped.
        let size = if self.escaped { bytes.len() + bytes.iter().filter(|&&b| b == b'"' || b == b'\\').count() } else { bytes.len() };
        if size > limit {
            return Err(bad(format!("response exceeds {limit} bytes once escaped")));
        }
        self.remaining = self.remaining.saturating_sub(size + 1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_bytes_include_json_escaping_and_accept_exact_boundary() {
        assert_eq!(encode_with_limit(&json!("abc"), 5).unwrap(), b"\"abc\"");
        assert!(encode_with_limit(&json!("abc"), 4).is_err());
        assert!(encode_with_limit(&json!("\n\n"), 5).is_err());
        assert!(encode_with_limit(&Value::Null, 0).is_err());
    }

    #[test]
    fn oversized_reply_is_one_complete_error_with_matching_id() {
        let reply = json!({"id": 7, "ok": true, "result": "x".repeat(MAX_RESPONSE_BYTES)});
        let mut out = Vec::new();
        write_reply(&mut out, &reply).unwrap();
        assert!(out.len() < 1024);
        assert_eq!(out.iter().filter(|&&byte| byte == b'\n').count(), 1);
        let error: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(error["id"], 7);
        assert_eq!(error["ok"], false);
        assert!(error["error"].as_str().unwrap().contains("operation may have completed"));
    }

    #[test]
    fn batch_cannot_retain_more_than_the_aggregate_reply_budget() {
        let mut budget = BatchReplyBudget { remaining: 10, escaped: false };
        budget.charge(&json!("abc")).unwrap();
        assert!(budget.charge(&json!("abc")).is_err());
        assert_eq!(budget.remaining, 4);
    }

    #[test]
    fn escaped_budget_charges_the_size_inside_a_json_string() {
        let value = json!("a\"b");
        let mut budget = BatchReplyBudget { remaining: 11, escaped: true };
        budget.charge(&value).unwrap();
        assert_eq!(budget.remaining, 0);
        let mut budget = BatchReplyBudget { remaining: 10, escaped: true };
        assert!(budget.charge(&value).is_err());
        assert!(BatchReplyBudget { remaining: 10, escaped: false }.charge(&value).is_ok());
    }
}
