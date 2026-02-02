use std::path::Path;
use std::process::Command;

/// Detected IDE type
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IdeType {
    VSCode,
    Cursor,
    JetBrains,
    None,
}

/// Detect which IDE is running
pub fn detect_ide() -> IdeType {
    // Check for Cursor (fork of VSCode) - check IPC hook path for "Cursor"
    if std::env::var("CURSOR_CHANNEL").is_ok() 
        || std::env::var("TERM_PROGRAM").map(|v| v.contains("Cursor")).unwrap_or(false)
        || std::env::var("VSCODE_IPC_HOOK").map(|v| v.contains("Cursor")).unwrap_or(false)
    {
        return IdeType::Cursor;
    }
    
    // Check for VSCode
    if std::env::var("VSCODE_IPC_HOOK").is_ok() 
        || std::env::var("TERM_PROGRAM").map(|v| v == "vscode").unwrap_or(false) 
        || std::env::var("VSCODE_GIT_IPC_HANDLE").is_ok()
    {
        return IdeType::VSCode;
    }
    
    // Check for JetBrains IDEs
    if std::env::var("JETBRAINS_REMOTE_RUN").is_ok()
        || std::env::var("TERMINAL_EMULATOR").map(|v| v.contains("JetBrains")).unwrap_or(false)
    {
        return IdeType::JetBrains;
    }
    
    IdeType::None
}

/// Show diff in native IDE
pub fn show_diff_in_ide(file_path: &Path, old_content: &str, new_content: &str) -> bool {
    let ide = detect_ide();
    
    // Create temp file with old content
    let temp_dir = std::env::temp_dir();
    let file_name = file_path.file_name().unwrap_or_default().to_string_lossy();
    let old_file = temp_dir.join(format!("{}.orig", file_name));
    
    if std::fs::write(&old_file, old_content).is_err() {
        return false;
    }
    
    let result = match ide {
        IdeType::Cursor => {
            // Cursor uses same CLI as VSCode
            Command::new("cursor")
                .args(["--diff", &old_file.to_string_lossy(), &file_path.to_string_lossy()])
                .spawn()
        }
        IdeType::VSCode => {
            Command::new("code")
                .args(["--diff", &old_file.to_string_lossy(), &file_path.to_string_lossy()])
                .spawn()
        }
        IdeType::JetBrains => {
            // Try common JetBrains CLI tools
            let tools = ["idea", "webstorm", "pycharm", "goland", "clion", "rustrover"];
            let mut success = false;
            
            for tool in tools {
                if Command::new(tool)
                    .args(["diff", &old_file.to_string_lossy(), &file_path.to_string_lossy()])
                    .spawn()
                    .is_ok()
                {
                    success = true;
                    break;
                }
            }
            
            if success {
                Ok(std::process::Child::from(std::process::Command::new("true").spawn().unwrap()))
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No JetBrains CLI found"))
            }
        }
        IdeType::None => {
            // No IDE detected, skip
            return false;
        }
    };
    
    result.is_ok()
}

/// Open file in IDE at specific line
pub fn open_file_in_ide(file_path: &Path, line: Option<usize>) -> bool {
    let ide = detect_ide();
    
    let file_arg = if let Some(ln) = line {
        format!("{}:{}", file_path.display(), ln)
    } else {
        file_path.to_string_lossy().to_string()
    };
    
    let result = match ide {
        IdeType::Cursor => {
            Command::new("cursor")
                .args(["--goto", &file_arg])
                .spawn()
        }
        IdeType::VSCode => {
            Command::new("code")
                .args(["--goto", &file_arg])
                .spawn()
        }
        IdeType::JetBrains => {
            let tools = ["idea", "webstorm", "pycharm", "goland", "clion", "rustrover"];
            
            for tool in tools {
                if Command::new(tool)
                    .args(["--line", &line.unwrap_or(1).to_string(), &file_path.to_string_lossy()])
                    .spawn()
                    .is_ok()
                {
                    return true;
                }
            }
            return false;
        }
        IdeType::None => return false,
    };
    
    result.is_ok()
}
