use anyhow::{Context, bail};

/// Hands `target` (exe path, exe name on the PATH, or URI such as `steam://...`) to the
/// Windows shell, like double-clicking it. Returns once the process is started.
pub fn launch(target: &str) -> anyhow::Result<()> {
    let target = target.trim();
    if target.is_empty() {
        bail!("aucune cible de lancement");
    }
    opener::open(target).with_context(|| format!("impossible de lancer {target}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_target_is_rejected() {
        assert!(launch("  ").is_err());
    }
}
