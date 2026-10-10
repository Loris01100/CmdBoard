//! Favorites: apps pinned to a slot from 1 to 9 (`:pin`, `p` on the dashboard) and
//! launched from any screen with `Alt+<slot>` (plan section 7).

use anyhow::bail;

use super::{App, MsgKind, Outcome};
use crate::command::PinChange;
use crate::storage::models::AppEntry;

/// Favorite slots, `Alt+1` to `Alt+9`.
pub const SLOTS: u8 = 9;

impl App {
    /// `:pin`: lists the favorites, or puts an app in a slot or out of it.
    pub(super) fn pin_command(&mut self, change: Option<PinChange>, app: Option<&str>) -> Outcome {
        let (Some(change), Some(app)) = (change, app) else {
            return Ok(Some((self.list_pins(), MsgKind::Info)));
        };
        let (id, name, current) = {
            let entry = self.app_named(app)?;
            (entry.id, entry.name.clone(), entry.pin)
        };
        let slot = match change {
            PinChange::Slot(slot) => Some(slot),
            PinChange::Off => None,
            PinChange::Toggle if current.is_some() => None,
            PinChange::Toggle => match self.free_slot() {
                Some(slot) => Some(slot),
                None => bail!(t!("pin.full")),
            },
        };
        if slot.is_none() && current.is_none() {
            return Ok(Some((t!("pin.not_pinned", name), MsgKind::Info)));
        }
        self.db.set_pin(id, slot)?;
        self.reload()?;
        let text = match slot {
            Some(slot) => t!("pin.set", name, slot),
            None => t!("pin.removed", name),
        };
        Ok(Some((text, MsgKind::Success)))
    }

    /// `Alt+<slot>`: launches the app pinned there.
    pub(super) fn launch_pin(&self, slot: u8) -> Outcome {
        match self.pinned(slot) {
            Some(entry) => self.launch(&entry.name.clone()),
            None => Ok(Some((t!("pin.empty", slot), MsgKind::Info))),
        }
    }

    pub fn pinned(&self, slot: u8) -> Option<&AppEntry> {
        self.apps.iter().find(|a| a.pin == Some(slot))
    }

    fn free_slot(&self) -> Option<u8> {
        (1..=SLOTS).find(|&slot| self.pinned(slot).is_none())
    }

    /// "1 Steam · 2 Hades", or how to pin one.
    fn list_pins(&self) -> String {
        let pins: Vec<String> = (1..=SLOTS)
            .filter_map(|slot| Some(format!("{slot} {}", self.pinned(slot)?.name)))
            .collect();
        if pins.is_empty() {
            return t!("pin.none");
        }
        t!("pin.list", items = pins.join(" · "))
    }
}
