//! Play time from before `CmdBoard` (plan section 11): what Steam counted per game, read
//! from `userdata\<account>\config\localconfig.vdf`, so a new user does not start at
//! level 1 with hundreds of hours played.

use std::collections::HashMap;

use super::scan::{quoted, steam_dir};

/// Minutes Steam counted per app id, every account of this PC together: the most any
/// account played, as accounts own games separately. `None` without Steam, or if no
/// account's file could be read.
pub fn steam_minutes() -> Option<HashMap<u32, u64>> {
    let userdata = steam_dir()?.join("userdata");
    let mut minutes: Option<HashMap<u32, u64>> = None;
    for account in std::fs::read_dir(userdata).ok()?.flatten() {
        let path = account.path().join(r"config\localconfig.vdf");
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let all = minutes.get_or_insert_with(HashMap::new);
        for (id, played) in parse(&text) {
            let best = all.entry(id).or_default();
            *best = (*best).max(played);
        }
    }
    minutes
}

/// The Steam app id a target launches: `steam://rungameid/<id>` (what the picker adds)
/// or `steam://run/<id>`.
pub fn steam_app_id(target: &str) -> Option<u32> {
    let lower = target.trim().to_ascii_lowercase();
    let rest = lower
        .strip_prefix("steam://rungameid/")
        .or_else(|| lower.strip_prefix("steam://run/"))?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// `"Playtime"` of each `Software\Valve\Steam\apps\<id>` block, in minutes. Keys ignore
/// case, as Steam has written both `apps` and `Apps`.
fn parse(vdf: &str) -> HashMap<u32, u64> {
    let mut path: Vec<String> = Vec::new();
    // A key alone on its line names the block the next `{` opens.
    let mut key = None;
    let mut minutes = HashMap::new();
    for line in vdf.lines() {
        match line.trim() {
            "{" => path.push(key.take().unwrap_or_default()),
            "}" => {
                path.pop();
            }
            _ => match quoted(line).as_slice() {
                [name] => key = Some(name.to_ascii_lowercase()),
                [name, value] if name.eq_ignore_ascii_case("playtime") => {
                    if let [.., software, valve, steam, apps, id] = path.as_slice()
                        && [software, valve, steam, apps] == ["software", "valve", "steam", "apps"]
                        && let (Ok(id), Ok(played)) = (id.parse(), value.parse())
                    {
                        minutes.insert(id, played);
                    }
                }
                _ => {}
            },
        }
    }
    minutes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_playtime_of_steam_apps_only() {
        let vdf = r#""UserLocalConfigStore"
{
	"friends"
	{
		"apps"
		{
			"10"
			{
				"Playtime"		"999"
			}
		}
	}
	"Software"
	{
		"Valve"
		{
			"Steam"
			{
				"Apps"
				{
					"7"
					{
						"cloud"
						{
							"last_sync_state"		"synchronized"
						}
					}
					"1245620"
					{
						"LastPlayed"		"1709416976"
						"Playtime"		"48000"
						"Playtime2wks"		"120"
					}
					"570"
					{
						"cloud"
						{
							"used_files"		"0"
						}
						"Playtime"		"266"
					}
					"bad"
					{
						"Playtime"		"1"
					}
				}
			}
		}
	}
}"#;
        let minutes = parse(vdf);
        assert_eq!(minutes, HashMap::from([(1_245_620, 48_000), (570, 266)]));
    }

    #[test]
    fn steam_targets_give_their_app_id() {
        let cases = [
            ("steam://rungameid/1245620", Some(1_245_620)),
            ("STEAM://run/570/", Some(570)),
            ("steam://rungameid/", None),
            ("steam://open/games", None),
            (r"C:\Games\elden ring.exe", None),
        ];
        for (target, id) in cases {
            assert_eq!(steam_app_id(target), id, "{target}");
        }
    }
}
