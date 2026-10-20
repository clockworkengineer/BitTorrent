use bittorrent_rs::announcer::{Announcer, AnnouncerEnum, AnnouncerFactory};
use bittorrent_rs::announcer_registry::{
    AnnouncerBuilder, AnnouncerRegistry,
};
use bittorrent_rs::error::BitTorrentError;
use bittorrent_rs::tracker::{
    AnnounceResponse, ScrapeResponse, TrackerAnnounceContext, TrackerEvent,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Mock announcer implementing the Announcer trait for custom schemes.
#[derive(Debug, Clone, Default)]
struct MockCustomAnnouncer {
    announce_calls: Arc<AtomicUsize>,
}

impl Announcer for MockCustomAnnouncer {
    fn announce(
        &mut self,
        _context: &TrackerAnnounceContext,
    ) -> Result<AnnounceResponse, BitTorrentError> {
        self.announce_calls.fetch_add(1, Ordering::Relaxed);
        let mut resp = AnnounceResponse::default();
        resp.complete = 42;
        resp.incomplete = 10;
        Ok(resp)
    }

    fn scrape(
        &mut self,
        _context: &TrackerAnnounceContext,
    ) -> Result<ScrapeResponse, BitTorrentError> {
        let mut resp = ScrapeResponse::default();
        resp.complete = 42;
        Ok(resp)
    }
}

/// Custom builder implementing AnnouncerBuilder for "mock://" scheme (OCP).
struct MockCustomAnnouncerBuilder {
    announce_calls: Arc<AtomicUsize>,
}

impl AnnouncerBuilder for MockCustomAnnouncerBuilder {
    fn build_announcer(&self, _url: &str) -> Result<Box<dyn Announcer>, BitTorrentError> {
        Ok(Box::new(MockCustomAnnouncer {
            announce_calls: self.announce_calls.clone(),
        }))
    }
}

fn create_dummy_context(tracker_url: &str) -> TrackerAnnounceContext {
    TrackerAnnounceContext {
        info_hash: vec![0xaa; 20],
        peer_id: "-TR2940-0123456789ab".to_string(),
        port: 6881,
        ip: "127.0.0.1".to_string(),
        compact: true,
        no_peer_id: true,
        key: None,
        tracker_id: None,
        num_wanted: 10,
        tracker_url: tracker_url.to_string(),
        event: TrackerEvent::Started,
        interval: 0,
        min_interval: 0,
        downloaded: 0,
        uploaded: 0,
        left: 1000,
        ipv6: None,
        #[cfg(feature = "http-tracker")]
        http_client: std::sync::Arc::new(bittorrent_rs::UreqHttpClient),
    }
}

#[test]
fn test_announcer_registry_default_protocols() {
    let registry = AnnouncerRegistry::new_with_defaults();
    assert!(!registry.is_empty());

    // 1. HTTP/HTTPS support
    #[cfg(feature = "http-tracker")]
    {
        assert!(registry.create_announcer("http://tracker.example.com/announce").is_ok());
        assert!(registry.create_announcer("https://tracker.example.com/announce").is_ok());
        assert!(matches!(
            registry.create("http://tracker.example.com/announce"),
            Ok(AnnouncerEnum::Http(_))
        ));
    }

    // 2. UDP support
    assert!(registry.create_announcer("udp://tracker.openbittorrent.com:6969/announce").is_ok());
    assert!(matches!(
        registry.create("udp://tracker.openbittorrent.com:6969/announce"),
        Ok(AnnouncerEnum::Udp(_))
    ));

    // 3. WebTorrent WebSocket support
    #[cfg(feature = "webrtc")]
    {
        assert!(registry.create_announcer("ws://tracker.btorrent.xyz").is_ok());
        assert!(registry.create_announcer("wss://tracker.openwebtorrent.com").is_ok());
        assert!(matches!(
            registry.create("wss://tracker.openwebtorrent.com"),
            Ok(AnnouncerEnum::WebTorrent(_))
        ));
    }

    // 4. Unsupported protocol returns error cleanly
    let err = registry.create_announcer("ftp://tracker.invalid.com/announce");
    assert!(err.is_err());
    let err_enum = registry.create("unsupported://tracker.invalid.com/announce");
    assert!(err_enum.is_err());
}

#[test]
fn test_announcer_registry_open_for_extension_closed_for_modification() {
    let mut registry = AnnouncerRegistry::new_with_defaults();
    let initial_count = registry.len();

    let calls = Arc::new(AtomicUsize::new(0));
    let custom_builder = MockCustomAnnouncerBuilder {
        announce_calls: calls.clone(),
    };

    // Register a brand new scheme "mock://" without touching library code (OCP)
    registry.register("mock://", custom_builder);
    assert_eq!(registry.len(), initial_count + 1);

    // Create trait-object Announcer for "mock://"
    let mut announcer = registry
        .create_announcer("mock://tracker.internal/announce")
        .expect("builder succeeds");

    let dummy_ctx = create_dummy_context("mock://tracker.internal/announce");
    let resp = announcer.announce(&dummy_ctx).expect("announce succeeds");
    assert_eq!(resp.complete, 42);
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    // Create AnnouncerEnum for "mock://"
    let announcer_enum = registry
        .create("mock://tracker.internal/announce")
        .expect("enum builder succeeds");
    assert!(matches!(announcer_enum, AnnouncerEnum::Custom(_)));
}

#[test]
fn test_announcer_registry_override_existing_scheme() {
    let mut registry = AnnouncerRegistry::new_with_defaults();

    let calls = Arc::new(AtomicUsize::new(0));
    let mock_override = MockCustomAnnouncerBuilder {
        announce_calls: calls.clone(),
    };

    // Register custom handler over "http://"
    registry.register("http://", mock_override);

    let mut announcer = registry
        .create_announcer("http://tracker.override.com/announce")
        .expect("override builder succeeds");

    let dummy_ctx = create_dummy_context("http://tracker.override.com/announce");
    let resp = announcer.announce(&dummy_ctx).expect("announce succeeds");
    assert_eq!(resp.complete, 42);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn test_announcer_factory_global_registration() {
    let calls = Arc::new(AtomicUsize::new(0));
    let custom_builder = MockCustomAnnouncerBuilder {
        announce_calls: calls.clone(),
    };

    // Register through AnnouncerFactory into global registry
    AnnouncerFactory::register("globalmock://", custom_builder);

    let announcer_res = AnnouncerFactory::create_announcer("globalmock://swarm.local/announce");
    assert!(announcer_res.is_ok());

    let enum_res = AnnouncerFactory::create("globalmock://swarm.local/announce");
    assert!(matches!(enum_res, Ok(AnnouncerEnum::Custom(_))));
}
