use anyhow::Result;
use git2::{Repository, Signature, IndexAddOption};
use std::path::{Path, PathBuf};
use std::fs;

/// Checkpoint manager using a shadow git repository
pub struct CheckpointManager {
    /// Path to the workspace being tracked
    workdir: PathBuf,
    /// Shadow git repository for checkpoints
    repo: Repository,
    /// List of checkpoint commits (newest first)
    checkpoints: Vec<String>,
}

impl CheckpointManager {
    /// Initialize checkpoint manager for a workspace
    pub fn new(workdir: &Path) -> Result<Self> {
        let shadow_path = Self::shadow_path(workdir)?;
        
        // Create shadow directory if needed
        fs::create_dir_all(&shadow_path)?;
        
        // Initialize or open shadow repo
        let repo = if shadow_path.join(".git").exists() {
            Repository::open(&shadow_path)?
        } else {
            let repo = Repository::init(&shadow_path)?;
            
            // Configure repo
            let mut config = repo.config()?;
            config.set_str("user.name", "Forge Checkpoint")?;
            config.set_str("user.email", "checkpoint@forge.local")?;
            
            repo
        };
        
        // Load existing checkpoints
        let checkpoints = Self::load_checkpoints(&repo)?;
        
        Ok(Self {
            workdir: workdir.to_path_buf(),
            repo,
            checkpoints,
        })
    }
    
    /// Create a checkpoint of the current workspace state
    pub fn create(&mut self, message: &str) -> Result<String> {
        // Sync workspace files to shadow repo
        self.sync_to_shadow()?;
        
        // Stage all changes
        let mut index = self.repo.index()?;
        index.add_all(["."].iter(), IndexAddOption::DEFAULT, None)?;
        index.write()?;
        
        // Create commit
        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;
        let sig = Signature::now("Forge Checkpoint", "checkpoint@forge.local")?;
        
        let parent = self.repo.head().ok()
            .and_then(|h| h.peel_to_commit().ok());
        
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        
        let commit_id = self.repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            message,
            &tree,
            &parents,
        )?;
        
        let commit_str = commit_id.to_string();
        self.checkpoints.insert(0, commit_str.clone());
        
        // Keep only last 50 checkpoints
        if self.checkpoints.len() > 50 {
            self.checkpoints.truncate(50);
        }
        
        Ok(commit_str)
    }
    
    /// Restore workspace to a specific checkpoint
    pub fn restore(&self, checkpoint_id: &str) -> Result<()> {
        let commit = self.repo.find_commit(git2::Oid::from_str(checkpoint_id)?)?;
        let tree = commit.tree()?;
        
        // Checkout the tree to shadow repo
        self.repo.checkout_tree(tree.as_object(), None)?;
        self.repo.set_head_detached(commit.id())?;
        
        // Sync from shadow back to workspace
        self.sync_from_shadow()?;
        
        Ok(())
    }
    
    /// Undo to the previous checkpoint
    pub fn undo(&self) -> Result<()> {
        if self.checkpoints.len() < 2 {
            return Err(anyhow::anyhow!("No previous checkpoint to undo to"));
        }
        
        self.restore(&self.checkpoints[1])
    }
    
    /// List all checkpoints
    pub fn list(&self) -> &[String] {
        &self.checkpoints
    }
    
    /// Get diff between current state and a checkpoint
    pub fn diff(&self, checkpoint_id: &str) -> Result<String> {
        // Sync current state first
        let mut temp_manager = Self::new(&self.workdir)?;
        temp_manager.create("temp")?;
        
        let commit = self.repo.find_commit(git2::Oid::from_str(checkpoint_id)?)?;
        let old_tree = commit.tree()?;
        
        let head = self.repo.head()?.peel_to_commit()?;
        let new_tree = head.tree()?;
        
        let diff = self.repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)?;
        
        let mut output = String::new();
        diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            let prefix = match line.origin() {
                '+' => "+",
                '-' => "-",
                ' ' => " ",
                _ => "",
            };
            if let Ok(content) = std::str::from_utf8(line.content()) {
                output.push_str(prefix);
                output.push_str(content);
            }
            true
        })?;
        
        Ok(output)
    }
    
    // --- Private helpers ---
    
    fn shadow_path(workdir: &Path) -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("No home directory"))?;
        let workspace_hash = Self::hash_path(workdir);
        Ok(home.join(".forge").join("checkpoints").join(workspace_hash))
    }
    
    fn hash_path(path: &Path) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        path.to_string_lossy().hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
    
    fn load_checkpoints(repo: &Repository) -> Result<Vec<String>> {
        let mut checkpoints = Vec::new();
        
        if let Ok(head) = repo.head() {
            if let Ok(commit) = head.peel_to_commit() {
                let mut revwalk = repo.revwalk()?;
                revwalk.push(commit.id())?;
                
                for oid in revwalk.take(50) {
                    if let Ok(oid) = oid {
                        checkpoints.push(oid.to_string());
                    }
                }
            }
        }
        
        Ok(checkpoints)
    }
    
    fn sync_to_shadow(&self) -> Result<()> {
        let shadow_path = Self::shadow_path(&self.workdir)?;
        
        // Walk workspace and copy files
        for entry in walkdir::WalkDir::new(&self.workdir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let rel_path = entry.path().strip_prefix(&self.workdir)?;
            
            // Skip hidden files and common ignores
            if Self::should_ignore(rel_path) {
                continue;
            }
            
            let dest_path = shadow_path.join(rel_path);
            
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)?;
            }
            
            fs::copy(entry.path(), dest_path)?;
        }
        
        Ok(())
    }
    
    fn sync_from_shadow(&self) -> Result<()> {
        let shadow_path = Self::shadow_path(&self.workdir)?;
        
        // Walk shadow and copy back to workspace
        for entry in walkdir::WalkDir::new(&shadow_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let rel_path = entry.path().strip_prefix(&shadow_path)?;
            
            // Skip .git directory
            if rel_path.starts_with(".git") {
                continue;
            }
            
            let dest_path = self.workdir.join(rel_path);
            
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)?;
            }
            
            fs::copy(entry.path(), dest_path)?;
        }
        
        Ok(())
    }
    
    fn should_ignore(path: &Path) -> bool {
        let ignored = [
            ".git", "node_modules", "target", "__pycache__", 
            ".venv", "venv", ".pytest_cache", "dist", "build",
            ".forge", ".next", ".nuxt", "coverage",
        ];
        
        path.components().any(|c| {
            c.as_os_str().to_str()
                .map(|s| ignored.contains(&s) || s.starts_with('.'))
                .unwrap_or(false)
        })
    }
}
