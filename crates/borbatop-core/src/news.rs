//! The Arch Linux news feed (RSS) and the history of upgrades in `pacman.log`.

use crate::bytes::Bytes;
use crate::timestamps::{parse_log_timestamp, parse_rfc822};

const ITEM_OPEN: &str = "<item>";
const ITEM_CLOSE: &str = "</item>";
const CDATA_OPEN: &str = "<![CDATA[";
const CDATA_CLOSE: &str = "]]>";
const UPGRADE_MARK: &str = "[PACMAN] starting full system upgrade";
const MANUAL_MARKS: [&str; 2] = ["manual intervention", "requires manual"];
const LOW_FREE_SPACE: Bytes = Bytes::new(2 * 1_024 * 1_024 * 1_024);
const LOW_FREE_PERMILLE: u64 = 50;

/// One announcement of the news feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsItem {
    title: String,
    link: String,
    published: Option<u64>,
}

impl NewsItem {
    /// Headline.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Address of the full text.
    #[must_use]
    pub fn link(&self) -> &str {
        &self.link
    }

    /// When it was published, in seconds since the Unix epoch, when the feed says.
    #[must_use]
    pub const fn published(&self) -> Option<u64> {
        self.published
    }

    /// Whether the headline says an upgrade needs a manual step (Arch's own wording).
    #[must_use]
    pub fn needs_manual_intervention(&self) -> bool {
        let title = self.title.to_lowercase();
        MANUAL_MARKS.iter().any(|mark| title.contains(mark))
    }
}

/// The items of the feed, newest first as the feed lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsFeed {
    items: Vec<NewsItem>,
    skipped: usize,
}

impl NewsFeed {
    /// Every item that could be read.
    #[must_use]
    pub fn items(&self) -> &[NewsItem] {
        &self.items
    }

    /// Items that had no title or link, counted instead of dropped silently.
    #[must_use]
    pub const fn skipped(&self) -> usize {
        self.skipped
    }

    /// The items published after `since`. An item with no date is kept: better to show an
    /// announcement twice than to hide one.
    ///
    /// # Arguments
    ///
    /// * `since` - seconds since the Unix epoch; `None` keeps everything
    #[must_use]
    pub fn published_after(&self, since: Option<u64>) -> Vec<&NewsItem> {
        self.items
            .iter()
            .filter(|item| match (since, item.published) {
                (Some(since), Some(published)) => published > since,
                (None, _) | (Some(_), None) => true,
            })
            .collect()
    }
}

/// Parses the RSS feed of `archlinux.org/feeds/news/`.
///
/// The reader looks only for `<item>` blocks and their `<title>`, `<link>` and `<pubDate>`, and
/// understands `CDATA` and the common character references.
///
/// # Arguments
///
/// * `xml` - the feed, as text
#[must_use]
pub fn parse_news_feed(xml: &str) -> NewsFeed {
    let mut items = Vec::new();
    let mut skipped = 0_usize;
    let mut rest = xml;
    while let Some(start) = rest.find(ITEM_OPEN) {
        let after = rest
            .get(start.saturating_add(ITEM_OPEN.len())..)
            .unwrap_or_default();
        let Some(end) = after.find(ITEM_CLOSE) else {
            break;
        };
        let block = after.get(..end).unwrap_or_default();
        match (tag_text(block, "title"), tag_text(block, "link")) {
            (Some(title), Some(link)) if !title.is_empty() => items.push(NewsItem {
                title,
                link,
                published: tag_text(block, "pubDate").and_then(|date| parse_rfc822(&date)),
            }),
            _ => skipped = skipped.saturating_add(1),
        }
        rest = after
            .get(end.saturating_add(ITEM_CLOSE.len())..)
            .unwrap_or_default();
    }
    NewsFeed { items, skipped }
}

/// The text of the first `<name>…</name>` in `block`, without CDATA wrappers and entities.
fn tag_text(block: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = block.find(&open)?.checked_add(open.len())?;
    let inner = block.get(start..)?;
    let raw = inner.get(..inner.find(&close)?)?.trim();
    let raw = raw
        .strip_prefix(CDATA_OPEN)
        .and_then(|text| text.strip_suffix(CDATA_CLOSE))
        .unwrap_or(raw);
    Some(unescape(raw).trim().to_owned())
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// When the last full system upgrade started, from the text of `pacman.log`.
///
/// # Arguments
///
/// * `log` - contents of `/var/log/pacman.log`
///
/// # Returns
///
/// Seconds since the Unix epoch, or `None` when the log has no full upgrade (or it is unreadable).
///
/// # Examples
///
/// ```
/// use borbatop_core::last_full_upgrade;
///
/// let log = "[2026-10-07T11:59:58-0300] [PACMAN] starting full system upgrade\n[2026-10-27T00:06:21-0300] [PACMAN] starting full system upgrade\n";
/// assert_eq!(last_full_upgrade(log), Some(1_793_070_381));
/// ```
#[must_use]
pub fn last_full_upgrade(log: &str) -> Option<u64> {
    log.lines()
        .rev()
        .find(|line| line.contains(UPGRADE_MARK))
        .and_then(|line| {
            let stamp = line.strip_prefix('[')?.split_once(']')?.0;
            parse_log_timestamp(stamp)
        })
}

/// Whether a filesystem is short of room for an upgrade: under 2 GiB free, or under 5% free.
///
/// # Arguments
///
/// * `free` - space available to a normal user
/// * `total` - size of the filesystem
#[must_use]
pub fn is_low_on_space(free: Bytes, total: Bytes) -> bool {
    free < LOW_FREE_SPACE || free.permille_of(total) < LOW_FREE_PERMILLE
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    const FEED: &str = r#"<?xml version="1.0"?><rss><channel><title>Arch</title><item><title>Plasma 7 &amp; Qt update requires manual intervention</title><link>https://archlinux.org/news/plasma/</link><pubDate>Wed, 21 Oct 2026 19:29:36 +0000</pubDate></item><item><title><![CDATA[glibc 2.45 & friends]]></title><link>https://archlinux.org/news/glibc/</link><pubDate>Mon, 05 Oct 2026 10:00:00 +0000</pubDate></item><item><title></title><link>x</link></item><item><title>No link</title></item><item><title>No date</title><link>https://archlinux.org/news/nodate/</link></item></channel></rss>"#;

    #[test]
    fn items_come_out_with_title_link_and_date() {
        let feed = parse_news_feed(FEED);
        let titles: Vec<&str> = feed.items().iter().map(NewsItem::title).collect();
        assert_eq!(
            titles,
            [
                "Plasma 7 & Qt update requires manual intervention",
                "glibc 2.45 & friends",
                "No date"
            ]
        );
        assert_eq!(feed.items()[0].link(), "https://archlinux.org/news/plasma/");
        assert_eq!(feed.items()[0].published(), Some(1_792_610_976));
        assert_eq!(feed.items()[2].published(), None);
        assert_eq!(feed.skipped(), 2);
    }

    #[test]
    fn manual_intervention_is_spotted_in_the_headline() {
        let feed = parse_news_feed(FEED);
        let flags: Vec<bool> = feed
            .items()
            .iter()
            .map(NewsItem::needs_manual_intervention)
            .collect();
        assert_eq!(flags, [true, false, false]);
    }

    #[test]
    fn only_news_after_the_last_upgrade_are_listed_and_undated_ones_are_kept() {
        let feed = parse_news_feed(FEED);
        let since_mid_october = feed.published_after(Some(1_791_385_198 + 10 * 86_400));
        let titles: Vec<&str> = since_mid_october.iter().map(|item| item.title()).collect();
        assert_eq!(
            titles,
            [
                "Plasma 7 & Qt update requires manual intervention",
                "No date"
            ]
        );
        assert_eq!(feed.published_after(None).len(), 3);
    }

    #[test]
    fn broken_feeds_give_nothing_instead_of_panicking() {
        for text in [
            "",
            "<rss>",
            "<item><title>x",
            "<item></item>",
            "not xml at all",
        ] {
            let feed = parse_news_feed(text);
            assert_eq!(feed.items(), []);
        }
    }

    #[test]
    fn the_last_full_upgrade_is_the_newest_one_in_the_log() {
        let log = "[2026-10-07T11:59:58-0300] [PACMAN] starting full system upgrade\n[2026-10-07T12:00:10-0300] [ALPM] upgraded linux\n[2026-10-27T00:06:21-0300] [PACMAN] starting full system upgrade\n[2026-10-27T00:09:00-0300] [ALPM] upgraded glibc\n";
        assert_eq!(last_full_upgrade(log), Some(1_793_070_381));
        assert_eq!(
            last_full_upgrade("[2026-10-07T11:59:58-0300] [ALPM] installed x\n"),
            None
        );
        assert_eq!(last_full_upgrade(""), None);
        assert_eq!(
            last_full_upgrade("[garbage] [PACMAN] starting full system upgrade\n"),
            None
        );
    }

    #[test]
    fn low_space_is_under_two_gib_or_under_five_percent() {
        let gib = 1_024 * 1_024 * 1_024;
        assert!(is_low_on_space(Bytes::new(gib), Bytes::new(100 * gib)));
        assert!(is_low_on_space(Bytes::new(3 * gib), Bytes::new(100 * gib)));
        assert!(!is_low_on_space(
            Bytes::new(10 * gib),
            Bytes::new(100 * gib)
        ));
        assert!(!is_low_on_space(Bytes::new(3 * gib), Bytes::new(20 * gib)));
    }
}
