use std::{fs, io::Write, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct LlmProfile {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FirecrawlProfile {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct CredentialStore {
    #[serde(default)]
    pub llm: Vec<LlmProfile>,
    #[serde(default)]
    pub firecrawl: Vec<FirecrawlProfile>,
    pub active_llm: Option<String>,
    pub active_firecrawl: Option<String>,
}

impl CredentialStore {
    pub fn path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("", "", "deep")
            .ok_or_else(|| anyhow!("unable to resolve the application configuration directory"))?;
        Ok(dirs.config_dir().join("credentials.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let bytes = fs::read(&path)
            .with_context(|| format!("failed to read credentials from {}", path.display()))?;
        serde_json::from_slice(&bytes).context("failed to parse credentials")
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let temp = path.with_extension("json.tmp");
        let payload = serde_json::to_vec_pretty(self)?;
        let mut file = fs::File::create(&temp)
            .with_context(|| format!("failed to create {}", temp.display()))?;
        file.write_all(&payload)?;
        file.sync_all()?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
        }

        fs::rename(&temp, &path)
            .with_context(|| format!("failed to persist credentials to {}", path.display()))?;
        Ok(())
    }

    pub fn has_required_profiles(&self) -> bool {
        self.active_llm().is_some() && self.active_firecrawl().is_some()
    }

    pub fn active_llm(&self) -> Option<LlmProfile> {
        self.active_llm
            .as_ref()
            .and_then(|name| self.llm.iter().find(|profile| &profile.name == name))
            .cloned()
            .or_else(|| self.llm.first().cloned())
            .or_else(llm_from_env)
    }

    pub fn active_firecrawl(&self) -> Option<FirecrawlProfile> {
        self.active_firecrawl
            .as_ref()
            .and_then(|name| self.firecrawl.iter().find(|profile| &profile.name == name))
            .cloned()
            .or_else(|| self.firecrawl.first().cloned())
            .or_else(firecrawl_from_env)
    }

    pub fn add_llm(&mut self, profile: LlmProfile) {
        let name = profile.name.clone();
        if let Some(existing) = self.llm.iter_mut().find(|item| item.name == name) {
            *existing = profile;
        } else {
            self.llm.push(profile);
        }
        self.active_llm = Some(name);
    }

    pub fn add_firecrawl(&mut self, profile: FirecrawlProfile) {
        let name = profile.name.clone();
        if let Some(existing) = self.firecrawl.iter_mut().find(|item| item.name == name) {
            *existing = profile;
        } else {
            self.firecrawl.push(profile);
        }
        self.active_firecrawl = Some(name);
    }

    pub fn set_active_llm(&mut self, name: &str) -> Result<()> {
        if !self.llm.iter().any(|profile| profile.name == name) {
            return Err(anyhow!("unknown LLM profile: {name}"));
        }
        self.active_llm = Some(name.to_owned());
        Ok(())
    }

    pub fn set_active_firecrawl(&mut self, name: &str) -> Result<()> {
        if !self.firecrawl.iter().any(|profile| profile.name == name) {
            return Err(anyhow!("unknown Firecrawl profile: {name}"));
        }
        self.active_firecrawl = Some(name.to_owned());
        Ok(())
    }
}

fn llm_from_env() -> Option<LlmProfile> {
    let api_key = std::env::var("OPENAI_API_KEY").ok()?;
    Some(LlmProfile {
        name: "environment".to_owned(),
        base_url: std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned()),
        model: std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-5".to_owned()),
        api_key,
    })
}

fn firecrawl_from_env() -> Option<FirecrawlProfile> {
    let api_key = std::env::var("FIRECRAWL_API_KEY").ok()?;
    Some(FirecrawlProfile {
        name: "environment".to_owned(),
        base_url: std::env::var("FIRECRAWL_BASE_URL")
            .unwrap_or_else(|_| "https://api.firecrawl.dev".to_owned()),
        api_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_profiles_selects_them() {
        let mut store = CredentialStore::default();
        store.add_llm(LlmProfile {
            name: "primary".into(),
            base_url: "https://example.test/v1".into(),
            model: "model".into(),
            api_key: "secret".into(),
        });
        store.add_firecrawl(FirecrawlProfile {
            name: "crawl".into(),
            base_url: "https://example.test".into(),
            api_key: "secret".into(),
        });

        assert_eq!(store.active_llm.as_deref(), Some("primary"));
        assert_eq!(store.active_firecrawl.as_deref(), Some("crawl"));
    }
}
