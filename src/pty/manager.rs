#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::pty::buffer::ReadSlice;
use crate::pty::id::generate_id;
use crate::pty::session::PtySession;
use crate::types::{SessionInfo, SpawnOptions, VoxError};

#[derive(Clone)]
pub struct PtyManager {
    sessions: Arc<Mutex<HashMap<String, Arc<PtySession>>>>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn spawn(&self, opts: SpawnOptions) -> Result<String, VoxError> {
        let id = generate_id();
        let timeout = opts.timeout_seconds;
        let session = Arc::new(PtySession::spawn(&id, opts)?);
        self.sessions
            .lock()
            .unwrap()
            .insert(id.clone(), session.clone());

        if let Some(secs) = timeout {
            let s = session.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                if s.status() == crate::types::Status::Running {
                    s.mark_timed_out();
                    s.kill();
                }
            });
        }
        Ok(id)
    }

    fn get(&self, id: &str) -> Result<Arc<PtySession>, VoxError> {
        self.sessions
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| VoxError::NotFound(id.to_string()))
    }

    pub fn write(&self, id: &str, data: &str) -> Result<(), VoxError> {
        self.get(id)?.write(data)
    }

    pub fn read(
        &self,
        id: &str,
        offset: usize,
        limit: Option<usize>,
        raw: bool,
    ) -> Result<ReadSlice, VoxError> {
        Ok(self.get(id)?.read(offset, limit, raw))
    }

    pub async fn wait(
        &self,
        id: &str,
        timeout_seconds: Option<u64>,
    ) -> Result<SessionInfo, VoxError> {
        let s = self.get(id)?;
        s.wait(timeout_seconds).await?;
        Ok(s.info())
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .map(|s| s.info())
            .collect()
    }

    pub fn kill(&self, id: &str, cleanup: bool) -> Result<(), VoxError> {
        let s = self.get(id)?;
        s.kill();
        if cleanup {
            self.sessions.lock().unwrap().remove(id);
        }
        Ok(())
    }

    /// Kill every session and clear the map — called on server shutdown.
    pub fn reap_all(&self) {
        let mut map = self.sessions.lock().unwrap();
        for s in map.values() {
            s.kill();
        }
        map.clear();
    }
}

impl Default for PtyManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SpawnOptions;

    fn sleep_opts(secs: u32) -> SpawnOptions {
        #[cfg(windows)]
        let (command, args) = (
            "cmd".to_string(),
            vec!["/C".into(), format!("ping -n {} 127.0.0.1", secs + 1)],
        );
        #[cfg(not(windows))]
        let (command, args) = ("/bin/sleep".to_string(), vec![secs.to_string()]);
        SpawnOptions {
            command,
            args,
            workdir: None,
            env: vec![],
            title: None,
            notify_on_exit: false,
            timeout_seconds: None,
        }
    }

    #[tokio::test]
    async fn spawn_list_kill_roundtrip() {
        let m = PtyManager::new();
        let id = m.spawn(sleep_opts(30)).unwrap();
        assert_eq!(m.list().len(), 1);
        assert!(m.kill(&id, false).is_ok());
        m.reap_all();
        assert_eq!(m.list().len(), 0);
    }
}
