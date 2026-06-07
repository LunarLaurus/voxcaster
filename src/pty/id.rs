// Module-level allow: consumers arrive in later tasks. Remove when PtyManager uses generate_id().
#![allow(dead_code)]

use std::time::{SystemTime, UNIX_EPOCH};

/// Generate a session id "pty_<16 hex>". Uses time + a process-local counter
/// xored with a multiplier for entropy without an RNG dependency.
pub fn generate_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    let mix = t ^ (n.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    format!("pty_{:016x}", mix)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_are_prefixed_and_unique() {
        let a = generate_id();
        let b = generate_id();
        assert!(a.starts_with("pty_"));
        assert_eq!(a.len(), 4 + 16); // "pty_" + 16 hex chars
        assert_ne!(a, b);
    }
}
