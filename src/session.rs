//! Session persistence - save and resume conversations

use crate::api::{Message, Role};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// A saved session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub workdir: PathBuf,
    pub provider: String,
    pub model: String,
    pub title: String,
    pub messages: Vec<Message>,
}

impl Session {
    /// Create a new session
    pub fn new(workdir: PathBuf, provider: &str, model: &str) -> Self {
        let now = Utc::now();
        let id = format!("{}", now.format("%Y%m%d-%H%M%S"));
        
        Self {
            id,
            created_at: now,
            updated_at: now,
            workdir,
            provider: provider.to_string(),
            model: model.to_string(),
            title: String::new(),
            messages: Vec::new(),
        }
    }

    /// Generate title from first user message
    fn generate_title(&mut self) {
        if self.title.is_empty() {
            if let Some(msg) = self.messages.iter().find(|m| m.role == Role::User) {
                self.title = truncate(&msg.content, 60);
            }
        }
    }

    /// Update messages and save
    pub fn update(&mut self, messages: &[Message]) -> Result<()> {
        self.messages = messages.to_vec();
        self.updated_at = Utc::now();
        self.generate_title();
        self.save()
    }

    /// Get sessions directory
    fn sessions_dir() -> Result<PathBuf> {
        let dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("forge")
            .join("sessions");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Save session to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::sessions_dir()?.join(format!("{}.json", self.id));
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Load session by ID
    pub fn load(id: &str) -> Result<Self> {
        let path = Self::sessions_dir()?.join(format!("{}.json", id));
        let json = fs::read_to_string(path)?;
        let session: Session = serde_json::from_str(&json)?;
        Ok(session)
    }

    /// Load the most recent session for a workdir
    pub fn load_latest(workdir: &Path) -> Result<Option<Self>> {
        let sessions = Self::list()?;
        let workdir_canonical = workdir.canonicalize().unwrap_or_else(|_| workdir.to_path_buf());
        
        for info in sessions {
            if let Ok(session) = Self::load(&info.id) {
                let session_workdir = session.workdir.canonicalize()
                    .unwrap_or_else(|_| session.workdir.clone());
                if session_workdir == workdir_canonical {
                    return Ok(Some(session));
                }
            }
        }
        Ok(None)
    }

    /// List all sessions (most recent first)
    pub fn list() -> Result<Vec<SessionInfo>> {
        let dir = Self::sessions_dir()?;
        let mut sessions = Vec::new();

        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    if let Ok(json) = fs::read_to_string(&path) {
                        if let Ok(session) = serde_json::from_str::<Session>(&json) {
                            sessions.push(SessionInfo {
                                id: session.id,
                                title: session.title,
                                updated_at: session.updated_at,
                                workdir: session.workdir,
                                message_count: session.messages.len(),
                            });
                        }
                    }
                }
            }
        }

        // Sort by updated_at descending
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }

    /// Delete a session
    pub fn delete(id: &str) -> Result<()> {
        let path = Self::sessions_dir()?.join(format!("{}.json", id));
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

/// Summary info for listing sessions
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub updated_at: DateTime<Utc>,
    pub workdir: PathBuf,
    pub message_count: usize,
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.lines().next().unwrap_or(s); // First line only
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_create_save_load() {
        let workdir = PathBuf::from("/tmp/test-forge");
        let session = Session::new(workdir.clone(), "gemini", "gemini-2.5-flash");
        
        assert!(!session.id.is_empty());
        assert_eq!(session.provider, "gemini");
        assert_eq!(session.model, "gemini-2.5-flash");
        assert!(session.messages.is_empty());
        
        // Save
        session.save().unwrap();
        
        // Load
        let loaded = Session::load(&session.id).unwrap();
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.provider, session.provider);
        
        // Cleanup
        Session::delete(&session.id).unwrap();
    }

    #[test]
    fn test_session_list() {
        let sessions = Session::list().unwrap();
        // Just verify it doesn't crash
        assert!(sessions.len() >= 0);
    }
}
