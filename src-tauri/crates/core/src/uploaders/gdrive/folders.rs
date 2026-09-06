//! Date subfolders `YYYY/MM/DD_backup/` (Should-have, T-5.3; SPEC.md §6
//! "gdrive/ › folders.rs (Should)"; PRD.md RF-015 "Estrutura:
//! YYYY/MM/DD_backup/").
//!
//! [`FolderResolver::resolve`] walks/creates the three-level tree under
//! the configured root folder, one `files.list` + optional `files.create`
//! per level, and caches the resulting leaf (`DD_backup/`) folder id
//! keyed by date so a day with many uploads only pays for that walk once
//! (RF-015: "folder created once per day"). `mod.rs` calls this instead
//! of using `cfg.folder_id` directly whenever `cfg.date_subfolders` is
//! on — see [`super::GDriveUploader::parent_for_upload`] — for every
//! upload path *except* `test_connection`, which always probes the
//! configured root regardless of this setting.
//!
//! **Race-safety within the process**: [`FolderResolver::resolve`] holds
//! its `cache` lock (a [`tokio::sync::Mutex`], not `std::sync::Mutex` —
//! it needs to stay held across the `.await`s of the Drive round trips
//! below) for the *entire* walk of one date, not just the map
//! lookup/insert. That's deliberately coarse: it guarantees at most one
//! in-process caller ever creates a given day's folder tree — several
//! workers finishing uploads for "today" at the same moment all pile up
//! on this lock, and only the first actually calls Drive — at the cost
//! of also serializing resolves of *different* dates against each other
//! (never a real bottleneck: this runs once per day per process, not per
//! file). It says nothing about *other processes/machines* racing the
//! same Drive folder; that's out of scope for T-5.3.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Local, NaiveDate};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use super::super::{ClassifyContext, UploadError};
use super::auth::TokenProvider;
use super::upload::{classify_response, map_reqwest_error};

/// Resolves/creates the `YYYY/MM/DD_backup/` subfolder tree under one
/// root Drive folder, caching the resolved leaf id per date (RF-015).
pub struct FolderResolver {
    http: reqwest::Client,
    tokens: Arc<TokenProvider>,
    /// The configured destination folder (`cfg.folder_id`) — every date's
    /// `YYYY/` folder is created directly under this.
    root: String,
    api_base: String,
    /// `"YYYY-MM-DD"` → the day's `DD_backup/` folder id. Cleared whole
    /// (not per-entry) whenever [`Self::resolve`] is called with a date
    /// different from the last one seen — see module docs and
    /// [`Self::day`].
    cache: Mutex<HashMap<String, String>>,
    /// The date [`Self::resolve`] was last called with. `None` before the
    /// first call. A mismatch on the next call clears [`Self::cache`]
    /// wholesale before resolving — "midnight invalidation": once the
    /// day rolls over, yesterday's cached leaf is no longer useful and is
    /// dropped rather than left to grow the map forever.
    day: Mutex<Option<NaiveDate>>,
}

impl FolderResolver {
    /// `http`/`tokens`/`api_base` are the same client, token provider and
    /// Drive API base the owning [`super::GDriveUploader`] uses — this
    /// makes no network calls of its own beyond the ones
    /// [`Self::resolve`] issues.
    pub fn new(
        http: reqwest::Client,
        tokens: Arc<TokenProvider>,
        root: String,
        api_base: String,
    ) -> Self {
        Self {
            http,
            tokens,
            root,
            api_base,
            cache: Mutex::new(HashMap::new()),
            day: Mutex::new(None),
        }
    }

    /// Today's date in local time (RF-015's `YYYY/MM/DD_backup/` is a
    /// local-calendar-day boundary, same as the night-mode window in
    /// `config::NightModeConfig` — both are user-facing "today", not
    /// UTC).
    pub fn today_local() -> NaiveDate {
        Local::now().date_naive()
    }

    /// Returns the Drive id of `date`'s `DD_backup/` folder, creating any
    /// of `YYYY/`, `MM/`, `DD_backup/` that don't already exist under
    /// [`Self::root`]. Cached per date (see [`Self::cache`]) — a second
    /// call with the same `date` returns instantly with no Drive calls.
    pub async fn resolve(&self, date: NaiveDate) -> Result<String, UploadError> {
        {
            let mut day = self.day.lock().await;
            if *day != Some(date) {
                self.cache.lock().await.clear();
                *day = Some(date);
            }
        }

        let key = date.format("%Y-%m-%d").to_string();

        // Held for the whole walk below, not just this lookup/insert —
        // see the module-level "race-safety" note.
        let mut cache = self.cache.lock().await;
        if let Some(leaf) = cache.get(&key) {
            return Ok(leaf.clone());
        }

        let year = date.format("%Y").to_string();
        let month = date.format("%m").to_string();
        let day_folder = format!("{}_backup", date.format("%d"));

        let mut parent = self.root.clone();
        for name in [year.as_str(), month.as_str(), day_folder.as_str()] {
            parent = self.find_or_create_child(name, &parent).await?;
        }

        cache.insert(key, parent.clone());
        Ok(parent)
    }

    /// One level of the walk: reuse `name` under `parent` if it already
    /// exists, else create it.
    async fn find_or_create_child(&self, name: &str, parent: &str) -> Result<String, UploadError> {
        if let Some(id) = self.find_child(name, parent).await? {
            return Ok(id);
        }
        self.create_child(name, parent).await
    }

    /// `files.list` scoped to `name` + `mimeType=folder` + `parent`
    /// (SPEC.md §6 T-5.3), returning the first match. `Ok(None)` means no
    /// such folder exists yet — the caller creates it.
    async fn find_child(&self, name: &str, parent: &str) -> Result<Option<String>, UploadError> {
        let token = self.tokens.token().await?;
        let q = folder_query(name, parent);
        let url = format!("{}/drive/v3/files", self.api_base);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .query(&[
                ("q", q.as_str()),
                ("fields", "files(id)"),
                ("supportsAllDrives", "true"),
                ("includeItemsFromAllDrives", "true"),
            ])
            .send()
            .await
            .map_err(|err| map_reqwest_error(&err))?;

        if !resp.status().is_success() {
            return Err(classify_response(resp, ClassifyContext::Api).await);
        }

        let value: Value = resp.json().await.map_err(|err| map_reqwest_error(&err))?;
        Ok(value
            .get("files")
            .and_then(|v| v.as_array())
            .and_then(|files| files.first())
            .and_then(|f| f.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()))
    }

    /// `POST /drive/v3/files` creating a folder named `name` under
    /// `parent` (SPEC.md §6 T-5.3).
    async fn create_child(&self, name: &str, parent: &str) -> Result<String, UploadError> {
        let token = self.tokens.token().await?;
        let url = format!("{}/drive/v3/files", self.api_base);
        let metadata = json!({
            "name": name,
            "mimeType": "application/vnd.google-apps.folder",
            "parents": [parent],
        });
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .query(&[("supportsAllDrives", "true"), ("fields", "id")])
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json; charset=UTF-8",
            )
            .body(metadata.to_string())
            .send()
            .await
            .map_err(|err| map_reqwest_error(&err))?;

        if !resp.status().is_success() {
            return Err(classify_response(resp, ClassifyContext::Api).await);
        }

        let value: Value = resp.json().await.map_err(|err| map_reqwest_error(&err))?;
        value
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| UploadError::Transient("Drive folder create returned no id".to_string()))
    }
}

/// Builds Drive's `q` filter for "the folder named `name` directly
/// inside `parent`, not trashed" (SPEC.md §6 T-5.3) — same escaping as
/// `upload::drive_query`, plus the `mimeType` clause that distinguishes
/// a folder from a same-named file.
fn folder_query(name: &str, parent: &str) -> String {
    let escape = |s: &str| s.replace('\\', "\\\\").replace('\'', "\\'");
    format!(
        "name = '{}' and '{}' in parents and mimeType = 'application/vnd.google-apps.folder' and trashed = false",
        escape(name),
        escape(parent)
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::config::GDriveConfig;
    use crate::throttle::Throttle;
    use crate::uploaders::UploadError;

    use super::super::auth::{parse_service_account, TokenProvider};
    use super::super::{GDriveOptions, GDriveUploader};
    use super::{FolderResolver, NaiveDate};

    /// Throwaway 2048-bit RSA key, PKCS8 PEM (see `gdrive::auth`'s test
    /// module for provenance) — copied here rather than re-exported, same
    /// reasoning as `gdrive::upload`'s test module.
    const TEST_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7Oyt3p8SoGEJP
bIsMFnQeQmYlR0NasbR3t6yCBKF6w/hzTMYKmWiBhxMoAtDWSpbNBwW0YoyEb9+m
hikW5GxE8wIBA0Xf1bBJuG50YA6wueZg/lDzcnKfEYGeD9VLoQC2fdN3AWvD1C33
UOowbMhZgslWxjHbRfICz989ZhZYB+RO6afajtdi2Fijju9sj+0v9GzhGZpL8RAa
NVjo5Dh6tVDJEOypTBqcx9IkeydD/eOQQA4ruhmsalHQWMfQUPJwyxEoj9dkvuj3
dSQcd4grxhniNPcPEKJ3xNp8JgbGxVVflJhU1Uu3GOMRtApwFuMwuPNQ7Fao0dYv
3DM4Q5CDAgMBAAECggEAGpEK72eyQKnEivGGCgPg8RmPKzNL7CwH7+7I1GAPgP8a
m1L694O3WkhjdaDEp4tzj18I4QS/bIaqmj3cybSaukPYS8go8Nmo3HpbeI7YDB0s
rSRDh0T83POby+TyPnt3bELkBQsCErTiMClejtrN/jQlGSIhQ+b5BTYgC93Bm4ww
7WZir/1E+5T8rqO3Ms3wmdC4dWTOQuDDWK0ljaLOqG0bW+t4eS6XHgn9CPS3DuJt
8wcmVlwPE2UXDkdv9r0DUk3D6FMcxU+2SdXehH/PqwzGk+NHrvn9lBlknF3vscs4
DvwATqvHoystxgI9LC52xIQbO+Uw0i82qUMUShrUTQKBgQDlIixIhRQpTCQGwiKc
/ZiDhO2ALMd5VxGY9xzsM+m2bTNSBy6ZgYEitZvy8txg3C0ExyyaBcqcRLeQ9gN7
DRvx9JEqsapjATfdqBt3JCv8AtjCxky44hkl3dFvjl5zLhu9O44Lcw0+2Y9KmrLz
+psqYkdCjoRUrYXA3pqzUOh9pwKBgQDRLzokZaKzBUGVgewLVp9DBgtDG09cw3m0
xbrVEUig0vt3EtpC/KcRAsRCLoVXlCzZssK+9qlKs63lUIMl29Uqftkm1t9LbDDI
Zgjj51wvM6DekTffx5XK8DEgzISsiBZtLPKyvmwUeVz9jVNb2U0K7n3iOG06T2Ok
ukYRbZAJxQKBgCU/vu8zIynrhNfMa5AV8ds/mtSBcxQYwXWahosnjVDow7UMEdlG
olWgLG/8ZzMf1/m0311Sn7NzwFvCgqJYaTiWR5snMsnRguF32K8vpC7dz5sqXYKY
zvnG66s0+8nBryS+L8NQutCC0baRG5JqJRtoyqjZPk39v4axKXkJKCJ1AoGBAKAD
lGJLLM3sc2K+Y6W4uVM3yF2pAmhfTzYtGuHpurjrK1jGnxcm1VV53E8T7wQzYKuW
xsn1PULbd2Y21Fudcc50AgBn1Z+IPzjMdHiBfk7NG32lcCxKLBd07N++Eq832o/h
FjYM2/g9bhi2htF3xCtcjAcESumT2RElPHwQZ2JRAoGAJMpVd75YW4YrgVdfV1Fl
AY61Vt5BYt7cXfwinvI5mc2BdT3Y7bzVuozxBmqbeo2ZyeCS+JEw3SeF5ckeBazT
Eqz5sfZq1hAiWB5WE9lc7DIUDxHEahm+RQwe4i939Sb5IRD9nvLRfvVFUL1OD/gu
h5sWg6OzGIQ6XQbasKq+W/8=
-----END PRIVATE KEY-----";

    const CLIENT_EMAIL: &str = "sync@my-project.iam.gserviceaccount.com";

    fn valid_sa_json() -> String {
        json!({
            "type": "service_account",
            "project_id": "my-project",
            "private_key_id": "abc123",
            "private_key": TEST_PRIVATE_KEY,
            "client_email": CLIENT_EMAIL,
            "client_id": "123456789",
            "token_uri": "https://oauth2.googleapis.com/token",
        })
        .to_string()
    }

    async fn mount_token_ok(token_server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "tok-1",
                "expires_in": 3600,
            })))
            .mount(token_server)
            .await;
    }

    async fn token_provider_for(token_server: &MockServer) -> Arc<TokenProvider> {
        let sa = parse_service_account(&valid_sa_json()).expect("valid service account json");
        TokenProvider::new(sa, Some(format!("{}/token", token_server.uri())))
            .await
            .expect("provider construction should succeed")
    }

    async fn mount_empty_list(api_server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "files": [] })))
            .mount(api_server)
            .await;
    }

    async fn resolver_for(token_server: &MockServer, api_server: &MockServer) -> FolderResolver {
        let tokens = token_provider_for(token_server).await;
        FolderResolver::new(
            reqwest::Client::new(),
            tokens,
            "root-folder".to_string(),
            api_server.uri(),
        )
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid calendar date")
    }

    // ---------------------------------------------------------------
    // 1. Concurrent resolves of the same date create each level once.
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn resolve_creates_the_tree_once_for_concurrent_callers_on_the_same_date() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        mount_token_ok(&token_server).await;

        // No folder exists yet at any level — every list lookup misses.
        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "files": [] })))
            .expect(3)
            .mount(&api_server)
            .await;

        for (needle, id) in [
            ("\"name\":\"2026\"", "year-id"),
            ("\"name\":\"09\"", "month-id"),
            ("\"name\":\"04_backup\"", "day-id"),
        ] {
            Mock::given(method("POST"))
                .and(path("/drive/v3/files"))
                .and(wiremock::matchers::body_string_contains(needle))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": id })))
                .expect(1)
                .mount(&api_server)
                .await;
        }

        let resolver = Arc::new(resolver_for(&token_server, &api_server).await);
        let target = date(2026, 9, 4);

        let mut handles = Vec::new();
        for _ in 0..3 {
            let resolver = resolver.clone();
            handles.push(tokio::spawn(async move { resolver.resolve(target).await }));
        }

        let mut ids = Vec::new();
        for handle in handles {
            ids.push(
                handle
                    .await
                    .expect("resolve task should not panic")
                    .expect("resolve should succeed"),
            );
        }

        assert!(
            ids.iter().all(|id| id == "day-id"),
            "all three concurrent callers should get the same cached leaf id, got {ids:?}"
        );

        // The `.expect(1)`/`.expect(3)` mocks above are verified on drop
        // (MockServer's Drop panics on unmet expectations), so reaching
        // here already proves exactly one create per level and at most
        // three list lookups total.
    }

    // ---------------------------------------------------------------
    // 2. Existing folders are reused — zero creates.
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn resolve_reuses_existing_folders_without_creating_anything() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        mount_token_ok(&token_server).await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "files": [{ "id": "existing-leaf" }] })),
            )
            .expect(3)
            .mount(&api_server)
            .await;
        // Deliberately no POST mock mounted: if the implementation tried
        // to create anything, wiremock would reject the unmatched
        // request and `resolve` would return an `Err`, failing the
        // `.expect` below.

        let resolver = resolver_for(&token_server, &api_server).await;
        let leaf = resolver
            .resolve(date(2026, 9, 4))
            .await
            .expect("resolve should succeed by reusing existing folders");

        assert_eq!(leaf, "existing-leaf");
    }

    // ---------------------------------------------------------------
    // 3. A new date clears the whole cache (midnight invalidation).
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn resolve_clears_the_whole_cache_when_the_date_changes() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        mount_token_ok(&token_server).await;
        mount_empty_list(&api_server).await;
        Mock::given(method("POST"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "some-id" })))
            .mount(&api_server)
            .await;

        let resolver = resolver_for(&token_server, &api_server).await;
        let day1 = date(2026, 9, 4);
        let day2 = date(2026, 9, 5);

        resolver.resolve(day1).await.expect("first date resolves");
        {
            let cache = resolver.cache.lock().await;
            assert_eq!(cache.len(), 1);
            assert!(cache.contains_key("2026-09-04"));
        }

        resolver.resolve(day2).await.expect("second date resolves");
        let cache = resolver.cache.lock().await;
        assert_eq!(
            cache.len(),
            1,
            "the whole cache should be cleared on a date change, not appended to"
        );
        assert!(cache.contains_key("2026-09-05"));
        assert!(!cache.contains_key("2026-09-04"));
    }

    // ---------------------------------------------------------------
    // 4. A 403 on create maps to Auth.
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn resolve_maps_a_403_on_create_to_auth() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        mount_token_ok(&token_server).await;
        mount_empty_list(&api_server).await;

        Mock::given(method("POST"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "error": { "errors": [{ "reason": "insufficientFilePermissions" }], "message": "no" }
            })))
            .mount(&api_server)
            .await;

        let resolver = resolver_for(&token_server, &api_server).await;
        let err = resolver
            .resolve(date(2026, 9, 4))
            .await
            .expect_err("a 403 on create should not resolve successfully");

        assert!(matches!(err, UploadError::Auth(_)));
    }

    // ---------------------------------------------------------------
    // 5. `date_subfolders = false` never touches the resolver.
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn date_subfolders_false_never_calls_the_resolver() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        mount_token_ok(&token_server).await;

        let tokens = token_provider_for(&token_server).await;
        let cfg = GDriveConfig {
            enabled: true,
            folder_id: "root-folder".to_string(),
            date_subfolders: false,
            ..Default::default()
        };
        let uploader = GDriveUploader::new(
            cfg,
            tokens,
            CLIENT_EMAIL.to_string(),
            Throttle::new(0),
            GDriveOptions {
                api_base: api_server.uri(),
                upload_base: api_server.uri(),
                state_sink: None,
            },
        );

        assert!(
            uploader.resolver.is_none(),
            "no FolderResolver should be built when date_subfolders is off"
        );

        let parent = uploader
            .parent_for_upload()
            .await
            .expect("parent_for_upload should not need the network when there's no resolver");
        assert_eq!(parent, "root-folder");

        let requests = api_server.received_requests().await.unwrap_or_default();
        assert!(
            requests.is_empty(),
            "resolving the parent without a resolver must not touch the Drive API, got {requests:?}"
        );
    }
}
