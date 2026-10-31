//! Sounds on Splice, through DECKARD's referral link: the Help menu's entry and the
//! browser's "find it on Splice" lines open splice.com, and subscriptions started that
//! way pay DECKARD a commission. The link is an Impact tracking link; Impact lands the
//! visitor on the page named by its `u` parameter (a Splice search for what was
//! typed), or on Splice's front page without one.

/// DECKARD's Splice referral link.
pub const LINK: &str = "https://splice.sjv.io/c/8024859/3810236/49453";

/// Said wherever the link is shown.
pub const DISCLOSURE: &str = "Opens splice.com. DECKARD earns a commission on subscriptions started through this link.";

/// The link to open: to a Splice search for `search` when given, else Splice's front
/// page.
pub fn url(search: Option<&str>) -> String {
    match search.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => {
            let target = format!("https://splice.com/sounds/search?q={}", encode(s));
            format!("{LINK}?u={}", encode(&target))
        }
        None => LINK.to_string(),
    }
}

/// Open the link in the system's browser.
pub fn open(ctx: &eframe::egui::Context, search: Option<&str>) {
    ctx.open_url(eframe::egui::OpenUrl::new_tab(url(search)));
}

/// Percent-encode `s` for a URL query value: letters, digits and `-._~` stay.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out += &format!("%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The referral link keeps its code, and a search rides along as Impact's `u`
    /// parameter, encoded so Splice's own query survives.
    #[test]
    fn the_link_credits_deckard_and_can_land_on_a_search() {
        assert_eq!(url(None), "https://splice.sjv.io/c/8024859/3810236/49453");
        assert_eq!(url(Some("  ")), url(None));
        assert_eq!(
            url(Some("808 kick")),
            "https://splice.sjv.io/c/8024859/3810236/49453?u=https%3A%2F%2Fsplice.com%2Fsounds%2Fsearch%3Fq%3D808%2520kick"
        );
        assert_eq!(encode("ä&b"), "%C3%A4%26b");
    }
}
