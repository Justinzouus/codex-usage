use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingSettings {
    pub monthly_usd: f64,
    pub extra_credits_usd: f64,
    pub day: u32,
    pub time_zone: String,
}

impl BillingSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !self.monthly_usd.is_finite()
            || self.monthly_usd <= 0.0
            || self.monthly_usd > 1_000_000.0
            || !self.extra_credits_usd.is_finite()
            || self.extra_credits_usd < 0.0
            || self.extra_credits_usd > 1_000_000.0
            || !(1..=31).contains(&self.day)
            || ![
                "UTC",
                "Asia/Shanghai",
                "Asia/Tokyo",
                "America/Los_Angeles",
                "America/New_York",
                "Europe/London",
            ]
            .contains(&self.time_zone.as_str())
        {
            return Err("请填写有效的订阅金额、额外额度、扣款日与时区。".into());
        }
        Ok(())
    }
}

pub fn load(path: &Path) -> Result<Option<BillingSettings>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("本地账期设置无法读取。".into()),
    };
    let settings: BillingSettings =
        serde_json::from_slice(&bytes).map_err(|_| "本地账期设置格式无效，请重新设置。")?;
    settings.validate()?;
    Ok(Some(settings))
}

pub fn save(path: &Path, settings: &BillingSettings) -> Result<(), String> {
    settings.validate()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "无法创建账期设置目录。")?;
    }
    let bytes = serde_json::to_vec_pretty(settings).map_err(|_| "无法序列化账期设置。")?;
    let temporary = path.with_extension("json.tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|_| "无法保存账期设置。")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| "无法保护账期设置。")?;
    }
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "无法保存账期设置。")?;
    fs::rename(temporary, path).map_err(|_| "无法更新账期设置。".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BillingSettings {
        BillingSettings {
            monthly_usd: 20.0,
            extra_credits_usd: 0.0,
            day: 1,
            time_zone: "UTC".into(),
        }
    }

    #[test]
    fn rejects_invalid_amounts_dates_and_timezones() {
        for settings in [
            BillingSettings {
                monthly_usd: 0.0,
                ..sample()
            },
            BillingSettings {
                monthly_usd: f64::NAN,
                ..sample()
            },
            BillingSettings {
                extra_credits_usd: -1.0,
                ..sample()
            },
            BillingSettings {
                day: 32,
                ..sample()
            },
            BillingSettings {
                time_zone: "invalid".into(),
                ..sample()
            },
        ] {
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn new_install_has_no_personal_baseline_and_settings_roundtrip_privately() {
        let directory =
            std::env::temp_dir().join(format!("codex-billing-test-{}", std::process::id()));
        let path = directory.join("billing-settings.json");
        assert_eq!(load(&path).unwrap(), None);
        save(&path, &sample()).unwrap();
        assert_eq!(load(&path).unwrap(), Some(sample()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::write(&path, "invalid").unwrap();
        assert!(load(&path).is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
