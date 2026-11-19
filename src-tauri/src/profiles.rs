use std::{fs, io::Write, path::PathBuf};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use uuid::Uuid;

use crate::error::{Result, StudioError};

const KEYRING_SERVICE: &str = "com.storyforge.studio.providers";

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub streaming: Option<String>,
    pub json_mode: Option<String>,
    pub usage: Option<String>,
    pub cancel: Option<String>,
    pub context_len: Option<String>,
    pub prompt_cache: Option<String>,
    pub multimodal: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub version: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub credential_ref: String,
    pub needs_key: bool,
    pub direct: bool,
    pub token_limit_field: String,
    pub capabilities: Capabilities,
    #[serde(default)]
    pub rates: Option<Rates>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedProfile {
    #[serde(flatten)]
    pub profile: Profile,
    pub has_key: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInput {
    pub id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub needs_key: bool,
    pub direct: bool,
    pub token_limit_field: String,
    pub rates: Option<Rates>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
}

pub struct ProfileStore {
    pub config_file: PathBuf,
}

pub fn completion_url(base: &str) -> Result<Url> {
    let mut url =
        Url::parse(base.trim()).map_err(|_| StudioError::new(400, "模型 Base URL 格式有误"))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || url.scheme() == "http" && loopback)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(StudioError::new(
            400,
            "模型地址须使用 HTTPS 或本机 HTTP，且不含凭据与查询参数",
        ));
    }
    let path = url.path().trim_end_matches('/');
    let suffix = if path.is_empty() {
        "/v1/chat/completions".into()
    } else if path.ends_with("/chat/completions") {
        path.to_owned()
    } else {
        format!("{path}/chat/completions")
    };
    url.set_path(&suffix);
    Ok(url)
}

impl ProfileStore {
    fn load(&self) -> Result<Vec<Profile>> {
        match fs::read(&self.config_file) {
            Ok(contents) => Ok(serde_json::from_slice(&contents)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(err) => Err(err.into()),
        }
    }

    fn save(&self, profiles: &[Profile]) -> Result<()> {
        let dir = self
            .config_file
            .parent()
            .ok_or_else(|| StudioError::new(500, "无法确定模型配置目录"))?;
        fs::create_dir_all(dir)?;
        let mut file = NamedTempFile::new_in(dir)?;
        file.write_all(&serde_json::to_vec_pretty(profiles)?)?;
        file.as_file().sync_all()?;
        file.persist(&self.config_file)
            .map_err(|err| StudioError::from(err.error))?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<ListedProfile>> {
        self.load()?
            .into_iter()
            .map(|profile| {
                let has_key = profile.secret()?.is_some();
                Ok(ListedProfile { profile, has_key })
            })
            .collect()
    }

    pub fn get(&self, id: &str) -> Result<Profile> {
        self.load()?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| StudioError::new(404, "找不到这个本机模型"))
    }

    pub fn upsert(&self, input: ProfileInput) -> Result<Profile> {
        let name = input.name.trim();
        let model = input.model.trim();
        if name.is_empty() || model.is_empty() || name.chars().count() > 80 || model.len() > 200 {
            return Err(StudioError::new(400, "请填写模型显示名和模型 ID"));
        }
        if !matches!(
            input.token_limit_field.as_str(),
            "max_tokens" | "max_completion_tokens"
        ) {
            return Err(StudioError::new(400, "请选择输出上限参数"));
        }
        let endpoint = completion_url(&input.base_url)?;
        if let Some(rates) = &input.rates {
            if [
                Some(rates.input),
                Some(rates.output),
                rates.cache_read,
                rates.cache_write,
            ]
            .into_iter()
            .flatten()
            .any(|rate| !rate.is_finite() || rate < 0.0)
            {
                return Err(StudioError::new(400, "模型费率须为非负数字"));
            }
        }
        let mut profiles = self.load()?;
        let index = if let Some(id) = input.id {
            Some(
                profiles
                    .iter()
                    .position(|p| p.id == id)
                    .ok_or_else(|| StudioError::new(404, "模型档案不存在"))?,
            )
        } else {
            None
        };
        let previous = index.map(|i| &profiles[i]);
        if let Some(previous) = previous {
            if completion_url(&previous.base_url)?.origin() != endpoint.origin()
                && previous.secret()?.is_some()
            {
                return Err(StudioError::new(
                    409,
                    "更换服务域名时请创建新的本机模型档案",
                ));
            }
        }
        let profile = Profile {
            id: previous
                .map(|p| p.id.clone())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            version: Uuid::new_v4().to_string(),
            credential_ref: previous
                .map(|p| p.credential_ref.clone())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            name: name.into(),
            model: model.into(),
            base_url: input.base_url.trim().into(),
            token_limit_field: input.token_limit_field,
            needs_key: input.needs_key,
            direct: input.direct,
            capabilities: Capabilities::default(),
            rates: input.rates,
        };
        if let Some(index) = index {
            profiles[index] = profile.clone();
        } else {
            profiles.push(profile.clone());
        }
        self.save(&profiles)?;
        Ok(profile)
    }

    pub fn update_capabilities(
        &self,
        profile: &Profile,
        capabilities: Capabilities,
    ) -> Result<Capabilities> {
        let mut profiles = self.load()?;
        let entry = profiles
            .iter_mut()
            .find(|p| p.id == profile.id && p.version == profile.version)
            .ok_or_else(|| StudioError::new(409, "模型配置已更改，请重新探测"))?;
        entry.capabilities = capabilities.clone();
        self.save(&profiles)?;
        Ok(capabilities)
    }

    pub fn remove(&self, profile_id: &str) -> Result<()> {
        let mut profiles = self.load()?;
        let index = profiles
            .iter()
            .position(|p| p.id == profile_id)
            .ok_or_else(|| StudioError::new(404, "模型档案不存在"))?;
        let profile = profiles.remove(index);
        profile.delete_secret()?;
        self.save(&profiles)?;
        Ok(())
    }
}

impl Profile {
    fn credential(&self) -> Result<keyring::Entry> {
        keyring::Entry::new(KEYRING_SERVICE, &self.credential_ref)
            .map_err(|_| StudioError::new(500, "无法访问系统凭据库"))
    }

    pub fn secret(&self) -> Result<Option<String>> {
        match self.credential()?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(StudioError::new(500, "无法读取系统凭据")),
        }
    }

    pub fn credential_for_request(&self) -> Result<Option<String>> {
        if !self.needs_key {
            return Ok(None);
        }
        Ok(Some(self.secret()?.ok_or_else(|| {
            StudioError::new(400, "请先保存该模型的 API 凭据")
        })?))
    }

    pub fn set_secret(&self, secret: &str) -> Result<()> {
        let secret = secret.trim();
        if secret.is_empty() || !secret.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(StudioError::new(400, "请在系统窗口输入有效的模型密钥"));
        }
        self.credential()?
            .set_password(secret)
            .map_err(|_| StudioError::new(500, "无法写入系统凭据"))
    }

    pub fn delete_secret(&self) -> Result<()> {
        match self.credential()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(StudioError::new(500, "无法删除系统凭据")),
        }
    }
}

#[cfg(windows)]
pub fn prompt_secret(profile: &Profile) -> Result<bool> {
    use std::{mem::size_of, ptr::null_mut};
    use windows_sys::Win32::Security::Credentials::{
        CredUIPromptForCredentialsW, CREDUI_FLAGS_ALWAYS_SHOW_UI, CREDUI_FLAGS_DO_NOT_PERSIST,
        CREDUI_FLAGS_GENERIC_CREDENTIALS, CREDUI_INFOW,
    };

    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<_>>();
    let caption = wide("StoryForge · 模型 API 密钥");
    let message = wide(&format!(
        "在密码框粘贴「{}」的 API 密钥。用户名无需修改。",
        profile.name
    ));
    let target = wide("StoryForge");
    let mut username = [0u16; 513];
    let name = wide("StoryForge");
    username[..name.len()].copy_from_slice(&name);
    let mut password = [0u16; 1025];
    let info = CREDUI_INFOW {
        cbSize: size_of::<CREDUI_INFOW>() as u32,
        hwndParent: null_mut(),
        pszMessageText: message.as_ptr(),
        pszCaptionText: caption.as_ptr(),
        hbmBanner: null_mut(),
    };
    let result = unsafe {
        CredUIPromptForCredentialsW(
            &info,
            target.as_ptr(),
            null_mut(),
            0,
            username.as_mut_ptr(),
            username.len() as u32,
            password.as_mut_ptr(),
            password.len() as u32,
            null_mut(),
            CREDUI_FLAGS_GENERIC_CREDENTIALS
                | CREDUI_FLAGS_DO_NOT_PERSIST
                | CREDUI_FLAGS_ALWAYS_SHOW_UI,
        )
    };
    if result == 1223 {
        return Ok(false);
    }
    if result != 0 {
        return Err(StudioError::new(500, "无法打开系统凭据窗口"));
    }
    let length = password
        .iter()
        .position(|&ch| ch == 0)
        .unwrap_or(password.len());
    let key = String::from_utf16_lossy(&password[..length]);
    password.fill(0);
    profile.set_secret(&key)?;
    Ok(true)
}

#[cfg(not(windows))]
pub fn prompt_secret(_: &Profile) -> Result<bool> {
    Err(StudioError::new(501, "此版本的凭据输入窗口面向 Windows"))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn windows_credential_roundtrip_does_not_write_secret_to_profile() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore {
            config_file: dir.path().join("profiles.json"),
        };
        let profile = store
            .upsert(ProfileInput {
                id: None,
                name: "fixture".into(),
                model: "fixture".into(),
                base_url: "http://127.0.0.1:41001/v1".into(),
                needs_key: true,
                direct: true,
                token_limit_field: "max_tokens".into(),
                rates: None,
            })
            .unwrap();
        assert!(!store.list().unwrap()[0].has_key);
        let test_secret = format!("fixture-{}", Uuid::new_v4());
        profile.set_secret(&test_secret).unwrap();
        let retrieved = profile.secret();
        let contains_key = std::fs::read_to_string(&store.config_file)
            .unwrap()
            .contains(&test_secret);
        profile.delete_secret().unwrap();
        assert_eq!(retrieved.unwrap().as_deref(), Some(test_secret.as_str()));
        assert!(!contains_key);
        assert!(profile.secret().unwrap().is_none());
    }
}
