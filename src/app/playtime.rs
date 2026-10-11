//! Steam play time from before `CmdBoard` (plan section 11), given once per app as XP:
//! at startup for every Steam app not imported yet, and as soon as one is added.

use std::collections::HashMap;

use super::sessions::XpAnim;
use super::{App, MsgKind};
use crate::core::goals::format_hm;
use crate::launcher::playtime;
use crate::storage::PastPlaytime;

/// Reads Steam's minutes per app id, `None` without Steam.
pub type SteamPlaytime = fn() -> Option<HashMap<u32, u64>>;

impl App {
    /// At startup, in the UI only: Steam's own count from now on.
    pub fn attach_steam_playtime(&mut self) {
        self.steam_playtime = playtime::steam_minutes;
    }

    /// At startup: imports every Steam app not imported yet, filling their XP bars.
    /// Returns what to tell the user, if any time was imported.
    pub fn import_steam_playtime(&mut self) -> Option<(String, MsgKind)> {
        let profile_before = self.profile.total_xp;
        let imported = match self.import_past(None) {
            Ok(imported) => imported,
            Err(e) => return Some((format!("{e:#}"), MsgKind::Error)),
        };
        let (secs, xp) = imported
            .iter()
            .fold((0, 0), |(s, x), (_, p)| (s + p.secs, x + p.xp));
        if xp == 0 {
            return None;
        }
        let frame = self.frame_count;
        for (app_id, past) in &imported {
            if let Some(entry) = self.find_app_by_id(*app_id) {
                let to = entry.total_xp;
                let from = to.saturating_sub(past.xp);
                self.xp_anims.insert(
                    *app_id,
                    XpAnim {
                        from,
                        to,
                        start: frame,
                    },
                );
            }
        }
        self.profile_anim = Some(XpAnim {
            from: profile_before,
            to: self.profile.total_xp,
            start: frame,
        });
        let text = t!("playtime.imported", time = format_hm(secs), xp);
        Some((text, MsgKind::Success))
    }

    /// Right after `app_id` is added: its Steam play time, with the usual level-up popup.
    /// Returns what to add to the "added" message.
    pub(super) fn import_added_playtime(&mut self, app_id: i64) -> Option<String> {
        let profile_before = self.profile.total_xp;
        let past = match self.import_past(Some(app_id)) {
            Ok(imported) => imported.into_iter().next()?.1,
            Err(e) => return Some(format!("{e:#}")),
        };
        if past.xp == 0 {
            return None;
        }
        self.on_xp_changed(app_id, 0, profile_before, past.xp);
        Some(t!(
            "playtime.added",
            time = format_hm(past.secs),
            xp = past.xp
        ))
    }

    /// Imports the Steam apps not imported yet (only `only`, if given), then reloads. A
    /// Steam app missing from Steam's files was never played: imported as nothing. An
    /// unreadable or empty file imports nothing, so the next start tries again.
    fn import_past(&mut self, only: Option<i64>) -> anyhow::Result<Vec<(i64, PastPlaytime)>> {
        let pending: Vec<(i64, u32)> = self
            .db
            .apps_without_past()?
            .into_iter()
            .filter(|(id, _)| only.is_none_or(|only| only == *id))
            .filter_map(|(id, target)| Some((id, playtime::steam_app_id(&target)?)))
            .collect();
        if pending.is_empty() {
            return Ok(Vec::new());
        }
        let Some(minutes) = (self.steam_playtime)().filter(|m| !m.is_empty()) else {
            return Ok(Vec::new());
        };
        let mut imported = Vec::new();
        for (app_id, steam_id) in pending {
            let played = minutes.get(&steam_id).copied().unwrap_or(0) * 60;
            if let Some(past) = self.db.import_past_playtime(app_id, played)? {
                imported.push((app_id, past));
            }
        }
        self.reload()?;
        Ok(imported)
    }
}
