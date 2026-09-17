//! The CalDAV wire protocol: discovery, listing, windowed queries, and
//! etag-guarded writes. Everything here is RFC 4791/6578-adjacent plumbing;
//! the ICS payloads are `ics.rs`'s business.
//!
//! Security rules (from the provider decision doc):
//! - **HTTPS only**, with an exception for addresses that can only be on the
//!   user's own wire — loopback, the private ranges, LAN-only names. See
//!   [`https_or_private`].
//! - **Redirects are followed same-host only.** Credentials never travel to
//!   a host the user did not name…
//! - …with one carve-out for hrefs (not redirects): a server may hand back
//!   absolute URLs on a sibling host — iCloud's principal lives on a
//!   `pNN-caldav.icloud.com` partition — and those are honoured only when
//!   the new host shares the registrable parent of the one the user gave.

use reqwest::{Method, StatusCode};
use url::{Host, Url};

#[derive(Debug, thiserror::Error)]
pub enum CalDavError {
    /// Wrong password, revoked app-specific password, locked account — the
    /// caller shows the reconnect banner, same as a dead Google token.
    #[error("the server rejected the credentials")]
    Unauthorized,
    /// The etag guard fired: someone else changed the resource first.
    #[error("the event changed on the server since it was loaded")]
    PreconditionFailed,
    #[error("the server answered {0}")]
    Http(StatusCode),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl CalDavError {
    pub fn needs_reauth(&self) -> bool {
        matches!(self, CalDavError::Unauthorized)
    }
}

/// One calendar (or task list) the server exposes.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredCalendar {
    /// Absolute URL of the collection.
    pub url: String,
    pub display_name: String,
    /// `#rrggbb` when the server publishes one (Apple ical namespace).
    pub color: Option<String>,
    pub ctag: Option<String>,
    pub supports_events: bool,
    pub supports_tasks: bool,
    /// True when the privilege set was published and lacks write.
    pub read_only: bool,
    /// The collection's own IANA zone, when it publishes one
    /// (`calendar-timezone`'s VTIMEZONE, read off its TZID) — what a bare
    /// all-day date resolves against.
    pub timezone: Option<String>,
}

/// One ICS resource inside a collection.
#[derive(Debug, Clone, PartialEq)]
pub struct Resource {
    /// Absolute URL.
    pub url: String,
    pub etag: Option<String>,
    pub ics: String,
}

/// An address book the account can read (#126).
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredAddressBook {
    /// Absolute URL of the collection.
    pub url: String,
    pub display_name: Option<String>,
}

struct AddressBookEntry {
    href: String,
    display_name: Option<String>,
}

pub struct CalDavClient {
    http: reqwest::Client,
    base: Url,
    username: String,
    password: String,
}

/// The refusal a plain-http address on the public internet earns.
///
/// A fixed literal with nothing interpolated, so it can be shown verbatim:
/// the user typed the address and gets to read why it was turned down. Pinned
/// by `errors::user_facing`'s allow-list on the app side.
pub const NOT_PRIVATE_HTTP: &str = "CalDAV needs an https:// address — plain http:// is accepted \
     only for a server on this machine or your own network";

/// Whether an address can only be reached across a wire the user owns.
///
/// Basic auth rides on every request, so cleartext http is a real exposure —
/// but only where the packets can be seen by someone else. This is therefore
/// an address test, not a preference: loopback, the RFC 1918 and RFC 4193
/// private ranges, link-local, and the names that cannot resolve outside a
/// LAN (`localhost`, mDNS `.local`, `.home.arpa`, and single-label hosts like
/// `nas`, which have no public TLD to be reached through).
///
/// Self-hosted Radicale is the case this exists for (issue #28): it ships
/// plain http on 5232, every other client on the box connects to it, and
/// refusing until the user fronts it with TLS is a wall rather than a
/// security control — the credential never leaves their own network.
///
/// Shared with WebCal feeds (`omacal-sync::webcal_feed`): feeds carry no
/// credentials but the same cleartext rule applies, so both refuse the same
/// literal (`NOT_PRIVATE_HTTP`).
pub fn is_private_host(host: &Host<&str>) -> bool {
    match host {
        Host::Ipv4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        // `is_unique_local` and `is_unicast_link_local` are still unstable, so
        // fc00::/7 and fe80::/10 are spelled out against the first group.
        Host::Ipv6(ip) => {
            ip.is_loopback()
                || (ip.segments()[0] & 0xfe00) == 0xfc00
                || (ip.segments()[0] & 0xffc0) == 0xfe80
        }
        Host::Domain(name) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            name == "localhost"
                || name.ends_with(".localhost")
                || name.ends_with(".local")
                || name.ends_with(".home.arpa")
                || !name.contains('.')
        }
    }
}

pub fn https_or_private(url: &Url) -> anyhow::Result<()> {
    if url.scheme() == "https" {
        return Ok(());
    }
    if url.scheme() == "http" && url.host().as_ref().is_some_and(is_private_host) {
        return Ok(());
    }
    // Deliberately interpolates nothing. The rejected URL may carry userinfo
    // (`http://user:password@host/`), and this string is allow-listed for
    // display in `errors::user_facing` — see `NOT_PRIVATE_HTTP` there.
    anyhow::bail!(NOT_PRIVATE_HTTP);
}

/// An unauthenticated HTTP client for public feed fetches (WebCal).
///
/// Separate from [`CalDavClient`]'s credentialed client on purpose: feeds
/// carry no credentials, so redirects may cross hosts (feeds commonly
/// redirect through CDNs); the CalDAV client follows same-host only.
/// The cleartext rule is identical — see [`https_or_private`].
pub fn anonymous_feed_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(anyhow::Error::from)
}

/// Whether `candidate` may receive the credentials given to `original`:
/// same host, or a sibling under the same registrable parent (last two
/// labels) — `caldav.icloud.com` → `p118-caldav.icloud.com`.
fn same_site(original: &Url, candidate: &Url) -> bool {
    let (Some(a), Some(b)) = (original.host_str(), candidate.host_str()) else {
        return false;
    };
    if a == b {
        return true;
    }
    let parent = |h: &str| {
        let labels: Vec<&str> = h.rsplitn(3, '.').collect();
        if labels.len() >= 2 { format!("{}.{}", labels[1], labels[0]) } else { h.to_string() }
    };
    parent(a) == parent(b)
}

const NS: &str = r#"xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/" xmlns:ic="http://apple.com/ns/ical/""#;
/// CardDAV's own namespace, alongside DAV's. Separate from [`NS`] because
/// only the address-book half speaks it, and a server that does not do
/// CardDAV at all should see no mention of it in a calendar request.
const CARD_NS: &str = r#"xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav""#;

impl CalDavClient {
    pub fn new(base_url: &str, username: &str, password: &str) -> anyhow::Result<Self> {
        let base = Url::parse(base_url.trim())?;
        https_or_private(&base)?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                // Same-host only: a redirect is the server's word, not the
                // user's, and credentials ride on every request.
                let same = attempt
                    .previous()
                    .last()
                    .map(|prev| prev.host_str() == attempt.url().host_str())
                    .unwrap_or(false);
                if same && attempt.previous().len() <= 5 {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()?;
        Ok(Self { http, base, username: username.to_string(), password: password.to_string() })
    }

    /// The URL this client was built for.
    pub fn base_url(&self) -> &str {
        self.base.as_str()
    }

    /// Resolves an href from a multistatus against `context`, refusing hosts
    /// outside the original site.
    fn resolve(&self, context: &Url, href: &str) -> anyhow::Result<Url> {
        let joined = context.join(href.trim())?;
        https_or_private(&joined)?;
        if !same_site(&self.base, &joined) {
            anyhow::bail!("server pointed outside its own site: {joined}");
        }
        Ok(joined)
    }

    async fn request(
        &self,
        method: &str,
        url: &Url,
        depth: Option<&str>,
        content_type: &str,
        body: String,
        extra: &[(&str, &str)],
    ) -> Result<(StatusCode, String, Option<String>), CalDavError> {
        let m = Method::from_bytes(method.as_bytes()).map_err(anyhow::Error::from)?;
        let mut req = self
            .http
            .request(m, url.clone())
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", content_type);
        if let Some(d) = depth {
            req = req.header("Depth", d);
        }
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let resp = req.body(body).send().await.map_err(anyhow::Error::from)?;
        let status = resp.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(CalDavError::Unauthorized);
        }
        if status == StatusCode::PRECONDITION_FAILED {
            return Err(CalDavError::PreconditionFailed);
        }
        let etag = resp
            .headers()
            .get("ETag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let text = resp.text().await.map_err(anyhow::Error::from)?;
        Ok((status, text, etag))
    }

    async fn propfind(&self, url: &Url, depth: &str, body: String) -> Result<String, CalDavError> {
        let (status, text, _) = self
            .request("PROPFIND", url, Some(depth), "application/xml; charset=utf-8", body, &[])
            .await?;
        if !status.is_success() {
            return Err(CalDavError::Http(status));
        }
        Ok(text)
    }

    /// Walks well-known → principal → calendar home → collections. Also the
    /// credential check: a wrong password fails here as `Unauthorized`.
    pub async fn discover(&self) -> Result<Vec<DiscoveredCalendar>, CalDavError> {
        // Principal. Try the given URL first; a server that answers the
        // well-known path with the principal directly also lands here.
        let body = format!(
            r#"<?xml version="1.0"?><d:propfind {NS}><d:prop><d:current-user-principal/></d:prop></d:propfind>"#
        );
        let text = match self.propfind(&self.base, "0", body.clone()).await {
            Ok(t) => t,
            Err(CalDavError::Http(_)) => {
                let wk = self.base.join("/.well-known/caldav").map_err(anyhow::Error::from)?;
                self.propfind(&wk, "0", body).await?
            }
            Err(e) => return Err(e),
        };
        let principal_href = first_href_prop(&text, "current-user-principal")
            .ok_or_else(|| anyhow::anyhow!("no current-user-principal in discovery answer"))?;
        let principal = self.resolve(&self.base, &principal_href)?;

        // Calendar home.
        let body = format!(
            r#"<?xml version="1.0"?><d:propfind {NS}><d:prop><c:calendar-home-set/></d:prop></d:propfind>"#
        );
        let text = self.propfind(&principal, "0", body).await?;
        let home_href = first_href_prop(&text, "calendar-home-set")
            .ok_or_else(|| anyhow::anyhow!("no calendar-home-set on the principal"))?;
        let home = self.resolve(&principal, &home_href)?;

        // The collections themselves.
        let body = format!(
            r#"<?xml version="1.0"?><d:propfind {NS}><d:prop>
                 <d:resourcetype/><d:displayname/><cs:getctag/>
                 <c:supported-calendar-component-set/><ic:calendar-color/><c:calendar-timezone/>
                 <d:current-user-privilege-set/>
               </d:prop></d:propfind>"#
        );
        let text = self.propfind(&home, "1", body).await?;
        let mut out = Vec::new();
        for entry in parse_collections(&text) {
            let url = match self.resolve(&home, &entry.href) {
                Ok(u) => u,
                Err(e) => {
                    tracing::warn!(%e, "skipping a collection with an unusable href");
                    continue;
                }
            };
            out.push(DiscoveredCalendar {
                url: url.to_string(),
                display_name: entry.display_name,
                color: entry.color,
                ctag: entry.ctag,
                supports_events: entry.supports_events,
                supports_tasks: entry.supports_tasks,
                read_only: entry.read_only,
                timezone: entry.timezone,
            });
        }
        Ok(out)
    }

    /// The address books this account can read (#126, RFC 6352).
    ///
    /// The same walk as [`Self::discover`] with one property changed:
    /// `addressbook-home-set` instead of the calendar's. A server with no
    /// CardDAV at all answers without the property, which is an empty list
    /// here rather than an error — plenty of CalDAV servers have no address
    /// books, and that is not a fault to report to anybody.
    pub async fn discover_address_books(&self) -> Result<Vec<DiscoveredAddressBook>, CalDavError> {
        let body = format!(
            r#"<?xml version="1.0"?><d:propfind {CARD_NS}><d:prop><d:current-user-principal/></d:prop></d:propfind>"#
        );
        let text = match self.propfind(&self.base, "0", body.clone()).await {
            Ok(t) => t,
            Err(CalDavError::Http(_)) => {
                let wk = self.base.join("/.well-known/carddav").map_err(anyhow::Error::from)?;
                self.propfind(&wk, "0", body).await?
            }
            Err(e) => return Err(e),
        };
        let Some(principal_href) = first_href_prop(&text, "current-user-principal") else {
            return Ok(Vec::new());
        };
        let principal = self.resolve(&self.base, &principal_href)?;

        let body = format!(
            r#"<?xml version="1.0"?><d:propfind {CARD_NS}><d:prop><card:addressbook-home-set/></d:prop></d:propfind>"#
        );
        let text = self.propfind(&principal, "0", body).await?;
        let Some(home_href) = first_href_prop(&text, "addressbook-home-set") else {
            return Ok(Vec::new());
        };
        let home = self.resolve(&principal, &home_href)?;

        let body = format!(
            r#"<?xml version="1.0"?><d:propfind {CARD_NS}><d:prop>
                 <d:resourcetype/><d:displayname/>
               </d:prop></d:propfind>"#
        );
        let text = self.propfind(&home, "1", body).await?;
        let mut out = Vec::new();
        for entry in parse_address_books(&text) {
            match self.resolve(&home, &entry.href) {
                Ok(url) => out.push(DiscoveredAddressBook {
                    url: url.to_string(),
                    display_name: entry.display_name,
                }),
                Err(e) => tracing::warn!(%e, "skipping an address book with an unusable href"),
            }
        }
        Ok(out)
    }

    /// Every card in an address book, as `(href, vCard)` pairs.
    ///
    /// `addressbook-query` with no filter is "all of them", which is what a
    /// suggestion list wants; the cards themselves come back in the same
    /// answer, so there is no second round trip per contact. A server that
    /// refuses the REPORT (some allow only PROPFIND) leaves an empty list
    /// rather than a failure: contacts are a convenience, and the calendar
    /// this account exists for must sync either way.
    pub async fn cards(&self, address_book_url: &str) -> Result<Vec<Resource>, CalDavError> {
        let url = Url::parse(address_book_url).map_err(anyhow::Error::from)?;
        let body = format!(
            r#"<?xml version="1.0"?><card:addressbook-query {CARD_NS}>
                 <d:prop><d:getetag/><card:address-data/></d:prop>
               </card:addressbook-query>"#
        );
        let (status, text, _) = self
            .request("REPORT", &url, Some("1"), "application/xml; charset=utf-8", body, &[])
            .await?;
        if !status.is_success() {
            return Err(CalDavError::Http(status));
        }
        let mut out = Vec::new();
        for r in parse_cards_multistatus(&text) {
            match self.resolve(&url, &r.href) {
                Ok(abs) => out.push(Resource { url: abs.to_string(), etag: r.etag, ics: r.ics }),
                Err(e) => tracing::warn!(%e, "skipping a card with an unusable href"),
            }
        }
        Ok(out)
    }

    /// The collection's current ctag — the cheap "anything changed?" probe.
    pub async fn ctag(&self, collection_url: &str) -> Result<Option<String>, CalDavError> {
        let url = Url::parse(collection_url).map_err(anyhow::Error::from)?;
        let body = format!(
            r#"<?xml version="1.0"?><d:propfind {NS}><d:prop><cs:getctag/></d:prop></d:propfind>"#
        );
        let text = self.propfind(&url, "0", body).await?;
        Ok(first_text_prop(&text, "getctag"))
    }

    async fn report(&self, url: &Url, body: String) -> Result<Vec<Resource>, CalDavError> {
        let (status, text, _) = self
            .request("REPORT", url, Some("1"), "application/xml; charset=utf-8", body, &[])
            .await?;
        if !status.is_success() {
            return Err(CalDavError::Http(status));
        }
        let mut out = Vec::new();
        for r in parse_resources(&text) {
            match self.resolve(url, &r.href) {
                Ok(abs) => out.push(Resource { url: abs.to_string(), etag: r.etag, ics: r.ics }),
                Err(e) => tracing::warn!(%e, "skipping a resource with an unusable href"),
            }
        }
        Ok(out)
    }

    /// Every VEVENT resource overlapping the window. The server expands
    /// nothing — a recurring series whose occurrences touch the window comes
    /// back as its master resource, which is exactly what the store wants.
    pub async fn events_in_window(
        &self,
        collection_url: &str,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<Vec<Resource>, CalDavError> {
        let url = Url::parse(collection_url).map_err(anyhow::Error::from)?;
        let body = format!(
            r#"<?xml version="1.0"?><c:calendar-query {NS}><d:prop><d:getetag/><c:calendar-data/></d:prop>
               <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT">
               <c:time-range start="{}" end="{}"/>
               </c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#,
            utc_stamp(start_ms),
            utc_stamp(end_ms),
        );
        self.report(&url, body).await
    }

    /// Every VTODO resource in the collection. No time filter: task lists
    /// are human-scale, and a due-date filter would hide undated tasks.
    pub async fn todos(&self, collection_url: &str) -> Result<Vec<Resource>, CalDavError> {
        let url = Url::parse(collection_url).map_err(anyhow::Error::from)?;
        let body = format!(
            r#"<?xml version="1.0"?><c:calendar-query {NS}><d:prop><d:getetag/><c:calendar-data/></d:prop>
               <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VTODO"/>
               </c:comp-filter></c:filter></c:calendar-query>"#
        );
        self.report(&url, body).await
    }

    /// Writes a resource. `etag: None` means create (`If-None-Match: *`, so
    /// a UID collision fails loudly instead of overwriting); `Some` guards an
    /// update. Returns the new etag when the server sends one — iCloud often
    /// does not, and the next sync reconciles.
    pub async fn put(
        &self,
        resource_url: &str,
        ics: &str,
        etag: Option<&str>,
    ) -> Result<Option<String>, CalDavError> {
        let guard: (&str, &str) = match etag {
            Some(t) => ("If-Match", t),
            None => ("If-None-Match", "*"),
        };
        self.put_with(resource_url, ics, &[guard]).await
    }

    /// Overwrites a resource that exists but whose etag is not known — the
    /// row of a server that answered an earlier PUT without one. Neither
    /// guard fits: `If-None-Match: *` is a create and fails with 412 on
    /// every such write, and there is no etag to match.
    pub async fn put_unguarded(&self, resource_url: &str, ics: &str) -> Result<Option<String>, CalDavError> {
        self.put_with(resource_url, ics, &[]).await
    }

    async fn put_with(
        &self,
        resource_url: &str,
        ics: &str,
        guard: &[(&str, &str)],
    ) -> Result<Option<String>, CalDavError> {
        let url = Url::parse(resource_url).map_err(anyhow::Error::from)?;
        https_or_private(&url).map_err(CalDavError::Other)?;
        let (status, _, new_etag) = self
            .request("PUT", &url, None, "text/calendar; charset=utf-8", ics.to_string(), guard)
            .await?;
        if !status.is_success() {
            return Err(CalDavError::Http(status));
        }
        Ok(new_etag)
    }

    /// A resource's current etag, asked with a depth-0 PROPFIND: what a
    /// write reads back when the server answered its PUT without one, so the
    /// next write can be guarded again.
    pub async fn etag_of(&self, resource_url: &str) -> Result<Option<String>, CalDavError> {
        let url = Url::parse(resource_url).map_err(anyhow::Error::from)?;
        https_or_private(&url).map_err(CalDavError::Other)?;
        let body = format!(
            r#"<?xml version="1.0"?><d:propfind {NS}><d:prop><d:getetag/></d:prop></d:propfind>"#
        );
        let xml = self.propfind(&url, "0", body).await?;
        let Ok(doc) = roxmltree::Document::parse(&xml) else { return Ok(None) };
        Ok(doc
            .descendants()
            .find(|n| n.is_element() && local(n.tag_name()) == "getetag")
            .and_then(|n| n.text())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string))
    }

    /// Deletes a resource, etag-guarded when one is known.
    pub async fn delete(&self, resource_url: &str, etag: Option<&str>) -> Result<(), CalDavError> {
        let url = Url::parse(resource_url).map_err(anyhow::Error::from)?;
        let extra: Vec<(&str, &str)> = etag.map(|t| ("If-Match", t)).into_iter().collect();
        let (status, _, _) = self
            .request("DELETE", &url, None, "text/plain", String::new(), &extra)
            .await?;
        // 404 on delete: already gone, which is what deleting wanted.
        if !status.is_success() && status != StatusCode::NOT_FOUND {
            return Err(CalDavError::Http(status));
        }
        Ok(())
    }
}

fn utc_stamp(ms: i64) -> String {
    let z = jiff::Timestamp::from_millisecond(ms)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
        .to_zoned(jiff::tz::TimeZone::UTC);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        z.year(), z.month(), z.day(), z.hour(), z.minute(), z.second()
    )
}

// --- multistatus parsing -----------------------------------------------------

fn local(name: roxmltree::ExpandedName) -> String {
    name.name().to_ascii_lowercase()
}

/// The first `<href>` nested under a prop with local name `prop_local`,
/// anywhere in the document — the shape both principal and home-set answers
/// share.
fn first_href_prop(xml: &str, prop_local: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    for node in doc.descendants() {
        if node.is_element() && local(node.tag_name()) == prop_local {
            for h in node.descendants() {
                if h.is_element() && local(h.tag_name()) == "href" {
                    let t = h.text().unwrap_or("").trim();
                    if !t.is_empty() {
                        return Some(t.to_string());
                    }
                }
            }
        }
    }
    None
}

fn first_text_prop(xml: &str, prop_local: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    for node in doc.descendants() {
        if node.is_element() && local(node.tag_name()) == prop_local {
            let t = node.text().unwrap_or("").trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

struct CollectionEntry {
    href: String,
    display_name: String,
    color: Option<String>,
    ctag: Option<String>,
    supports_events: bool,
    supports_tasks: bool,
    read_only: bool,
    timezone: Option<String>,
}

fn parse_collections(xml: &str) -> Vec<CollectionEntry> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.is_element() && local(n.tag_name()) == "response") {
        let href = resp
            .children()
            .find(|n| n.is_element() && local(n.tag_name()) == "href")
            .and_then(|n| n.text())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        if href.is_empty() {
            continue;
        }
        let mut is_calendar = false;
        let mut display_name = String::new();
        let mut color = None;
        let mut ctag = None;
        let mut comps: Vec<String> = Vec::new();
        let mut saw_comp_set = false;
        let mut saw_privileges = false;
        let mut can_write = false;
        let mut timezone = None;
        for node in resp.descendants().filter(|n| n.is_element()) {
            match local(node.tag_name()).as_str() {
                "resourcetype" => {
                    is_calendar = node
                        .children()
                        .any(|c| c.is_element() && local(c.tag_name()) == "calendar");
                }
                "displayname" => display_name = node.text().unwrap_or("").trim().to_string(),
                "calendar-color" => {
                    // Apple sends #RRGGBBAA; the store draws #rrggbb.
                    let raw = node.text().unwrap_or("").trim().to_string();
                    if raw.len() >= 7 && raw.starts_with('#') {
                        color = Some(raw[..7].to_ascii_lowercase());
                    }
                }
                "getctag" => {
                    let t = node.text().unwrap_or("").trim();
                    if !t.is_empty() {
                        ctag = Some(t.to_string());
                    }
                }
                "supported-calendar-component-set" => {
                    saw_comp_set = true;
                    for c in node.children().filter(|c| c.is_element() && local(c.tag_name()) == "comp") {
                        if let Some(name) = c.attribute("name") {
                            comps.push(name.to_ascii_uppercase());
                        }
                    }
                }
                "calendar-timezone" => {
                    // The value is a whole VTIMEZONE; its TZID is the answer.
                    timezone = node
                        .text()
                        .unwrap_or("")
                        .lines()
                        .find_map(|l| l.trim().strip_prefix("TZID:"))
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty());
                }
                "current-user-privilege-set" => {
                    saw_privileges = true;
                    can_write |= node
                        .descendants()
                        .any(|p| p.is_element() && matches!(local(p.tag_name()).as_str(), "write" | "write-content" | "all"));
                }
                _ => {}
            }
        }
        if !is_calendar {
            continue;
        }
        // A server that does not publish the component set gets the RFC
        // default reading: a calendar collection may hold anything.
        let supports_events = !saw_comp_set || comps.iter().any(|c| c == "VEVENT");
        let supports_tasks = !saw_comp_set || comps.iter().any(|c| c == "VTODO");
        if display_name.is_empty() {
            display_name = href.trim_end_matches('/').rsplit('/').next().unwrap_or("Calendar").to_string();
        }
        out.push(CollectionEntry {
            href,
            display_name,
            color,
            ctag,
            supports_events,
            supports_tasks,
            read_only: saw_privileges && !can_write,
            timezone,
        });
    }
    out
}

struct RawResource {
    href: String,
    etag: Option<String>,
    ics: String,
}

/// An address book from the home set: a collection whose `resourcetype`
/// carries CardDAV's `addressbook`.
fn parse_address_books(xml: &str) -> Vec<AddressBookEntry> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.is_element() && local(n.tag_name()) == "response") {
        let href = resp
            .children()
            .find(|n| n.is_element() && local(n.tag_name()) == "href")
            .and_then(|n| n.text())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let mut is_book = false;
        let mut display_name = None;
        for node in resp.descendants().filter(|n| n.is_element()) {
            match local(node.tag_name()).as_str() {
                "addressbook" => is_book = true,
                "displayname" => {
                    display_name = node.text().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
                }
                _ => {}
            }
        }
        if is_book && !href.is_empty() {
            out.push(AddressBookEntry { href, display_name });
        }
    }
    out
}

/// [`parse_resources`] for cards: the payload element is `address-data`
/// rather than `calendar-data`, and everything else is the same multistatus.
fn parse_cards_multistatus(xml: &str) -> Vec<RawResource> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.is_element() && local(n.tag_name()) == "response") {
        let href = resp
            .children()
            .find(|n| n.is_element() && local(n.tag_name()) == "href")
            .and_then(|n| n.text())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let mut etag = None;
        let mut card = String::new();
        for node in resp.descendants().filter(|n| n.is_element()) {
            match local(node.tag_name()).as_str() {
                "getetag" => etag = node.text().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string),
                "address-data" => card = node.text().unwrap_or("").to_string(),
                _ => {}
            }
        }
        if !href.is_empty() && !card.trim().is_empty() {
            out.push(RawResource { href, etag, ics: card });
        }
    }
    out
}

fn parse_resources(xml: &str) -> Vec<RawResource> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.is_element() && local(n.tag_name()) == "response") {
        let href = resp
            .children()
            .find(|n| n.is_element() && local(n.tag_name()) == "href")
            .and_then(|n| n.text())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let mut etag = None;
        let mut ics = String::new();
        for node in resp.descendants().filter(|n| n.is_element()) {
            match local(node.tag_name()).as_str() {
                "getetag" => etag = node.text().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string),
                "calendar-data" => ics = node.text().unwrap_or("").to_string(),
                _ => {}
            }
        }
        if !href.is_empty() && !ics.trim().is_empty() {
            out.push(RawResource { href, etag, ics });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn only_same_site_hosts_may_see_credentials() {
        let base = Url::parse("https://caldav.icloud.com/").unwrap();
        let ok = Url::parse("https://p118-caldav.icloud.com/123/principal/").unwrap();
        let bad = Url::parse("https://caldav.evil.com/").unwrap();
        assert!(same_site(&base, &ok));
        assert!(!same_site(&base, &bad));
    }

    #[test]
    fn plain_http_is_refused_on_the_open_internet() {
        assert!(CalDavClient::new("http://cal.example.com/", "u", "p").is_err());
        assert!(CalDavClient::new("https://cal.example.com/", "u", "p").is_ok());
    }

    /// Issue #28. A self-hosted server is named by LAN address or LAN name,
    /// and Radicale's own default is plain http on 5232 — every one of these
    /// keeps the credential on a wire the user already owns.
    #[test]
    fn plain_http_is_accepted_on_the_users_own_network() {
        for url in [
            "http://localhost:5232/",
            "http://127.0.0.1:5232/",
            "http://[::1]:5232/",
            "http://192.168.1.20:5232/",
            "http://10.0.0.4:5232/",
            "http://172.16.3.9:5232/",
            "http://169.254.7.7:5232/",
            "http://[fd00::1]:5232/",
            "http://[fe80::1]:5232/",
            "http://nas:5232/",
            "http://radicale.local:5232/",
            "http://dav.home.arpa:5232/",
        ] {
            assert!(CalDavClient::new(url, "u", "p").is_ok(), "refused {url}");
        }
    }

    /// The test is the address, not the spelling: a public host that merely
    /// reads as private is still public, and `192.168.1.20.example.com`
    /// resolves to whatever its owner wants.
    #[test]
    fn a_public_host_that_reads_as_private_is_still_refused() {
        for url in [
            "http://192.168.1.20.example.com/",
            "http://localhost.example.com/",
            "http://cal.notlocal/",
            "http://8.8.8.8/",
        ] {
            assert!(CalDavClient::new(url, "u", "p").is_err(), "accepted {url}");
        }
    }

    /// The refusal is shown to the user verbatim (it is allow-listed in
    /// `errors::user_facing`), so it must never quote the address back — a
    /// pasted URL can carry the password in its userinfo.
    #[test]
    fn the_scheme_refusal_does_not_repeat_the_address_back() {
        // `.err().expect(..)` rather than `expect_err`: `CalDavClient` holds a
        // password and deliberately has no `Debug`.
        let err = CalDavClient::new("http://someone:hunter2@cal.example.com/", "u", "p")
            .err()
            .expect("a public http address must be refused");
        let shown = err.to_string();
        assert_eq!(shown, NOT_PRIVATE_HTTP);
        assert!(!shown.contains("hunter2"), "userinfo leaked: {shown}");
    }

    const PRINCIPAL: &str = r#"<?xml version="1.0"?>
      <d:multistatus xmlns:d="DAV:"><d:response><d:href>/</d:href><d:propstat><d:prop>
      <d:current-user-principal><d:href>/principals/p/</d:href></d:current-user-principal>
      </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;

    const HOME: &str = r#"<?xml version="1.0"?>
      <d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response>
      <d:href>/principals/p/</d:href><d:propstat><d:prop>
      <c:calendar-home-set><d:href>/cals/p/</d:href></c:calendar-home-set>
      </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;

    const COLLECTIONS: &str = r#"<?xml version="1.0"?>
      <d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"
                     xmlns:cs="http://calendarserver.org/ns/" xmlns:ic="http://apple.com/ns/ical/">
      <d:response><d:href>/cals/p/</d:href><d:propstat><d:prop>
        <d:resourcetype><d:collection/></d:resourcetype></d:prop>
        <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
      <d:response><d:href>/cals/p/work/</d:href><d:propstat><d:prop>
        <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
        <d:displayname>Work</d:displayname>
        <ic:calendar-color>#FF2968FF</ic:calendar-color>
        <cs:getctag>ctag-1</cs:getctag>
        <c:supported-calendar-component-set><c:comp name="VEVENT"/></c:supported-calendar-component-set>
        <d:current-user-privilege-set><d:privilege><d:write/></d:privilege></d:current-user-privilege-set>
        </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
      <d:response><d:href>/cals/p/chores/</d:href><d:propstat><d:prop>
        <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
        <d:displayname>Chores</d:displayname>
        <c:supported-calendar-component-set><c:comp name="VTODO"/></c:supported-calendar-component-set>
        <d:current-user-privilege-set><d:privilege><d:read/></d:privilege></d:current-user-privilege-set>
        </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
      </d:multistatus>"#;

    #[tokio::test]
    async fn discovery_walks_principal_home_and_collections() {
        let server = MockServer::start().await;
        Mock::given(method("PROPFIND")).and(path("/"))
            .respond_with(ResponseTemplate::new(207).set_body_raw(PRINCIPAL, "application/xml"))
            .mount(&server).await;
        Mock::given(method("PROPFIND")).and(path("/principals/p/"))
            .respond_with(ResponseTemplate::new(207).set_body_raw(HOME, "application/xml"))
            .mount(&server).await;
        Mock::given(method("PROPFIND")).and(path("/cals/p/"))
            .respond_with(ResponseTemplate::new(207).set_body_raw(COLLECTIONS, "application/xml"))
            .mount(&server).await;

        let client = CalDavClient::new(&server.uri(), "user", "pass").unwrap();
        let cals = client.discover().await.unwrap();
        assert_eq!(cals.len(), 2, "the home collection itself is not a calendar");

        let work = &cals[0];
        assert_eq!(work.display_name, "Work");
        assert_eq!(work.color.as_deref(), Some("#ff2968"), "#RRGGBBAA trimmed and lowered");
        assert_eq!(work.ctag.as_deref(), Some("ctag-1"));
        assert!(work.supports_events && !work.supports_tasks);
        assert!(!work.read_only);

        let chores = &cals[1];
        assert!(chores.supports_tasks && !chores.supports_events);
        assert!(chores.read_only, "a read privilege alone means read-only");
    }

    /// The CardDAV walk (#126), which is the calendar's with one property
    /// changed. The home holds the collection itself and one address book;
    /// only the book comes back.
    #[tokio::test]
    async fn address_book_discovery_walks_principal_home_and_books() {
        const CARD_HOME: &str = r#"<?xml version="1.0"?>
          <d:multistatus xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">
          <d:response><d:href>/principals/p/</d:href><d:propstat><d:prop>
            <card:addressbook-home-set><d:href>/cards/p/</d:href></card:addressbook-home-set>
          </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        const BOOKS: &str = r#"<?xml version="1.0"?>
          <d:multistatus xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">
          <d:response><d:href>/cards/p/</d:href><d:propstat><d:prop>
            <d:resourcetype><d:collection/></d:resourcetype><d:displayname>home</d:displayname>
          </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
          <d:response><d:href>/cards/p/contacts/</d:href><d:propstat><d:prop>
            <d:resourcetype><d:collection/><card:addressbook/></d:resourcetype>
            <d:displayname>Contacts</d:displayname>
          </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
          </d:multistatus>"#;
        let server = MockServer::start().await;
        Mock::given(method("PROPFIND")).and(path("/"))
            .respond_with(ResponseTemplate::new(207).set_body_raw(PRINCIPAL, "application/xml"))
            .mount(&server).await;
        Mock::given(method("PROPFIND")).and(path("/principals/p/"))
            .respond_with(ResponseTemplate::new(207).set_body_raw(CARD_HOME, "application/xml"))
            .mount(&server).await;
        Mock::given(method("PROPFIND")).and(path("/cards/p/"))
            .respond_with(ResponseTemplate::new(207).set_body_raw(BOOKS, "application/xml"))
            .mount(&server).await;

        let client = CalDavClient::new(&server.uri(), "user", "pass").unwrap();
        let books = client.discover_address_books().await.unwrap();
        assert_eq!(books.len(), 1, "the home collection itself is not an address book");
        assert_eq!(books[0].display_name.as_deref(), Some("Contacts"));
        assert!(books[0].url.ends_with("/cards/p/contacts/"));
    }

    /// **A server with no CardDAV is not a failure.** Plenty of CalDAV
    /// servers have no address books, and the calendar this account exists
    /// for must sync regardless.
    #[tokio::test]
    async fn a_server_without_address_books_answers_with_none() {
        const NO_HOME: &str = r#"<?xml version="1.0"?>
          <d:multistatus xmlns:d="DAV:"><d:response><d:href>/principals/p/</d:href>
          <d:propstat><d:prop/><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat>
          </d:response></d:multistatus>"#;
        let server = MockServer::start().await;
        Mock::given(method("PROPFIND")).and(path("/"))
            .respond_with(ResponseTemplate::new(207).set_body_raw(PRINCIPAL, "application/xml"))
            .mount(&server).await;
        Mock::given(method("PROPFIND")).and(path("/principals/p/"))
            .respond_with(ResponseTemplate::new(207).set_body_raw(NO_HOME, "application/xml"))
            .mount(&server).await;
        let client = CalDavClient::new(&server.uri(), "user", "pass").unwrap();
        assert_eq!(client.discover_address_books().await.unwrap(), Vec::new());
    }

    /// The cards themselves ride the same answer as their etags, so a book
    /// of 300 contacts is one round trip and not 301.
    #[tokio::test]
    async fn an_address_book_query_returns_the_cards_themselves() {
        let body = r#"<?xml version="1.0"?>
          <d:multistatus xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">
          <d:response><d:href>/cards/p/contacts/ana.vcf</d:href><d:propstat><d:prop>
            <d:getetag>"v1"</d:getetag>
            <card:address-data>BEGIN:VCARD
VERSION:3.0
FN:Ana Petrova
EMAIL:ana@x.com
END:VCARD
</card:address-data>
          </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
          <d:response><d:href>/cards/p/contacts/empty.vcf</d:href><d:propstat><d:prop>
            <d:getetag>"v2"</d:getetag>
          </d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>
          </d:multistatus>"#;
        let server = MockServer::start().await;
        Mock::given(method("REPORT")).and(path("/cards/p/contacts/"))
            .respond_with(ResponseTemplate::new(207).set_body_raw(body, "application/xml"))
            .mount(&server).await;
        let client = CalDavClient::new(&server.uri(), "user", "pass").unwrap();
        let cards = client.cards(&format!("{}/cards/p/contacts/", server.uri())).await.unwrap();
        assert_eq!(cards.len(), 1, "a response carrying no card is not one");
        assert_eq!(cards[0].etag.as_deref(), Some("\"v1\""));
        assert!(cards[0].ics.contains("FN:Ana Petrova"));
        assert_eq!(crate::vcard::parse_cards(&cards[0].ics)[0].emails, vec!["ana@x.com".to_string()]);
    }

    #[tokio::test]
    async fn a_wrong_password_reads_as_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("PROPFIND"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server).await;
        let client = CalDavClient::new(&server.uri(), "user", "wrong").unwrap();
        assert!(matches!(client.discover().await, Err(CalDavError::Unauthorized)));
    }

    #[tokio::test]
    async fn a_calendar_query_returns_resources_with_etags() {
        let body = r#"<?xml version="1.0"?>
          <d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
          <d:response><d:href>/cals/p/work/e1.ics</d:href><d:propstat><d:prop>
            <d:getetag>"abc"</d:getetag>
            <c:calendar-data>BEGIN:VCALENDAR
BEGIN:VEVENT
UID:e1
DTSTART:20260817T090000Z
END:VEVENT
END:VCALENDAR</c:calendar-data>
          </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        let server = MockServer::start().await;
        Mock::given(method("REPORT")).and(path("/cals/p/work/")).and(header("Depth", "1"))
            .respond_with(ResponseTemplate::new(207).set_body_raw(body, "application/xml"))
            .mount(&server).await;

        let client = CalDavClient::new(&server.uri(), "u", "p").unwrap();
        let url = format!("{}/cals/p/work/", server.uri());
        let got = client.events_in_window(&url, 0, 1_000_000).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].etag.as_deref(), Some("\"abc\""));
        assert!(got[0].ics.contains("UID:e1"));
        assert!(got[0].url.ends_with("/cals/p/work/e1.ics"));
    }

    #[tokio::test]
    async fn puts_carry_the_right_guard_and_preconditions_surface() {
        let server = MockServer::start().await;
        Mock::given(method("PUT")).and(path("/cals/p/work/new.ics")).and(header("If-None-Match", "*"))
            .respond_with(ResponseTemplate::new(201).insert_header("ETag", "\"fresh\""))
            .mount(&server).await;
        Mock::given(method("PUT")).and(path("/cals/p/work/old.ics")).and(header("If-Match", "\"v1\""))
            .respond_with(ResponseTemplate::new(412))
            .mount(&server).await;

        let client = CalDavClient::new(&server.uri(), "u", "p").unwrap();
        let created = client
            .put(&format!("{}/cals/p/work/new.ics", server.uri()), "BEGIN:VCALENDAR\r\nEND:VCALENDAR", None)
            .await
            .unwrap();
        assert_eq!(created.as_deref(), Some("\"fresh\""));

        let conflict = client
            .put(&format!("{}/cals/p/work/old.ics", server.uri()), "x", Some("\"v1\""))
            .await;
        assert!(matches!(conflict, Err(CalDavError::PreconditionFailed)));
    }

    /// iCloud answers a PUT without an ETag. The etag is asked for, and a
    /// write whose etag is unknown goes out with no guard at all, never as a
    /// create.
    #[tokio::test]
    async fn an_etag_the_put_did_not_give_is_asked_for_and_unguarded_puts_carry_no_guard() {
        let server = MockServer::start().await;
        Mock::given(method("PROPFIND")).and(path("/cals/p/work/t.ics")).and(header("Depth", "0"))
            .respond_with(ResponseTemplate::new(207).set_body_string(
                r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>/cals/p/work/t.ics</d:href><d:propstat><d:prop><d:getetag>"v7"</d:getetag></d:prop></d:propstat></d:response></d:multistatus>"#,
            ))
            .mount(&server).await;
        Mock::given(method("PUT")).and(path("/cals/p/work/t.ics"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server).await;

        let client = CalDavClient::new(&server.uri(), "u", "p").unwrap();
        let href = format!("{}/cals/p/work/t.ics", server.uri());
        assert_eq!(client.etag_of(&href).await.unwrap().as_deref(), Some("\"v7\""));
        assert_eq!(client.put_unguarded(&href, "x").await.unwrap(), None);
        let puts = server.received_requests().await.unwrap();
        let put = puts.iter().find(|r| r.method.as_str() == "PUT").unwrap();
        assert!(put.headers.get("If-Match").is_none() && put.headers.get("If-None-Match").is_none());
    }

    #[tokio::test]
    async fn deleting_something_already_gone_is_success() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server).await;
        let client = CalDavClient::new(&server.uri(), "u", "p").unwrap();
        client.delete(&format!("{}/x.ics", server.uri()), None).await.unwrap();
    }
}
