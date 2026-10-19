use bittorrent_rs::{
    MagnetLink, MagnetUriBuilder, SequentialSelector, SessionConfig, TorrentSessionBuilder,
    internals::TorrentContext,
    network::webtorrent_injector::{DEFAULT_WEBTORRENT_TRACKERS, WebTorrentMagnetInjector},
};
use std::sync::Arc;

#[test]
fn test_magnet_link_parse_webseeds_and_v2_multihash() {
    let uri = "magnet:?xt=urn:btih:3f069123cd2052c93540c4908ef48df8f480393c\
&xt=urn:btmh:1220a591a6d40bf420404a011733cfb7b190d62c65bf0bcda32b57b277d9ad9f146e\
&dn=Big+Buck+Bunny\
&tr=wss%3A%2F%2Ftracker.openwebtorrent.com\
&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce\
&ws=https%3A%2F%2Fwebtorrent.io%2Ftorrents%2F\
&ws=https%3A%2F%2Fmirror.archive.org%2Fdownload%2Fbbb%2F\
&xs=https%3A%2F%2Fwebtorrent.io%2Ftorrents%2Fbig-buck-bunny.torrent";

    let magnet = MagnetLink::parse(uri).expect("Parse hybrid magnet link with webseeds");

    // Check info-hashes
    assert_eq!(magnet.info_hash.len(), 20);
    assert_eq!(magnet.info_hash[0], 0x3f);
    assert_eq!(magnet.info_hash[19], 0x3c);

    let v2 = magnet.v2_info_hash.as_ref().expect("v2 info hash present");
    assert_eq!(v2.len(), 32);
    assert_eq!(v2[0], 0xa5);
    assert_eq!(v2[31], 0x6e);

    // Check classification
    assert!(magnet.is_hybrid());
    assert!(magnet.is_v2());
    assert!(!magnet.is_v1_only());
    assert!(!magnet.is_v2_only());
    assert!(magnet.has_web_seeds());
    assert!(magnet.is_webtorrent());

    // Check display name
    assert_eq!(magnet.display_name.as_deref(), Some("Big Buck Bunny"));

    // Check trackers
    assert_eq!(magnet.trackers.len(), 2);
    let ws_trackers = magnet.websocket_trackers();
    assert_eq!(ws_trackers, vec!["wss://tracker.openwebtorrent.com"]);
    let udp_trackers = magnet.udp_trackers();
    assert_eq!(
        udp_trackers,
        vec!["udp://tracker.opentrackr.org:1337/announce"]
    );

    // Check web seeds & exact sources
    assert_eq!(magnet.web_seeds.len(), 2);
    assert_eq!(magnet.web_seeds[0], "https://webtorrent.io/torrents/");
    assert_eq!(
        magnet.web_seeds[1],
        "https://mirror.archive.org/download/bbb/"
    );
    assert_eq!(magnet.exact_sources.len(), 1);
    assert_eq!(
        magnet.exact_sources[0],
        "https://webtorrent.io/torrents/big-buck-bunny.torrent"
    );

    // Check round-trip serialization
    let serialized_uri = magnet.to_uri();
    assert!(serialized_uri.starts_with("magnet:?"));
    assert!(serialized_uri.contains("xt=urn:btih:3f069123cd2052c93540c4908ef48df8f480393c"));
    assert!(serialized_uri.contains(
        "xt=urn:btmh:1220a591a6d40bf420404a011733cfb7b190d62c65bf0bcda32b57b277d9ad9f146e"
    ));
    assert!(serialized_uri.contains("dn=Big%20Buck%20Bunny"));
    assert!(serialized_uri.contains("ws=https%3A%2F%2Fwebtorrent.io%2Ftorrents%2F"));

    let reparsed = MagnetLink::parse(&serialized_uri).expect("Reparse serialized magnet URI");
    assert_eq!(reparsed.info_hash, magnet.info_hash);
    assert_eq!(reparsed.v2_info_hash, magnet.v2_info_hash);
    assert_eq!(reparsed.display_name, magnet.display_name);
    assert_eq!(reparsed.trackers, magnet.trackers);
    assert_eq!(reparsed.web_seeds, magnet.web_seeds);
}

#[test]
fn test_pure_v2_magnet_link_parsing() {
    let uri = "magnet:?xt=urn:btmh:1220a591a6d40bf420404a011733cfb7b190d62c65bf0bcda32b57b277d9ad9f146e&dn=Pure+V2";
    let magnet = MagnetLink::parse(uri).expect("Parse pure v2 magnet link");

    assert!(magnet.is_v2());
    assert!(magnet.is_v2_only());
    assert!(!magnet.is_hybrid());
    assert_eq!(magnet.info_hash.len(), 20); // 20-byte truncated v2 prefix
    assert_eq!(
        &magnet.info_hash[..],
        &magnet.v2_info_hash.as_ref().unwrap()[..20]
    );

    let serialized = magnet.to_uri();
    assert!(serialized.contains("xt=urn:btmh:1220"));
}

#[test]
fn test_webtorrent_magnet_injector_extraction_and_categorization() {
    let uri = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567\
&dn=Test+Swarm\
&tr=udp%3A%2F%2Ftracker.coppersurfer.tk%3A6969\
&tr=http%3A%2F%2Ftracker.ubuntu.com%3A6969%2Fannounce\
&tr=wss%3A%2F%2Ftracker.btorrent.xyz\
&tr=ws%3A%2F%2Flocalhost%3A8000\
&ws=https%3A%2F%2Farchive.org%2Fdownload%2Ftest%2F\
&xs=https%3A%2F%2Fexample.com%2Ftest.torrent";

    let magnet = MagnetLink::parse(uri).expect("Parse test magnet");
    let report = WebTorrentMagnetInjector::extract_trackers(&magnet);

    assert_eq!(report.total_trackers, 4);
    assert!(report.has_webtorrent);
    assert_eq!(report.websocket_trackers.len(), 2);
    assert!(
        report
            .websocket_trackers
            .contains(&"wss://tracker.btorrent.xyz".to_string())
    );
    assert!(
        report
            .websocket_trackers
            .contains(&"ws://localhost:8000".to_string())
    );
    assert_eq!(report.udp_trackers.len(), 1);
    assert_eq!(report.http_trackers.len(), 1);
    assert_eq!(report.web_seeds.len(), 1);
    assert_eq!(report.web_seeds[0], "https://archive.org/download/test/");
    assert_eq!(report.exact_sources.len(), 1);
}

#[test]
fn test_webtorrent_magnet_injector_auto_injection_and_uri_rewrite() {
    let uri = "magnet:?xt=urn:btih:1122334455667788990011223344556677889900\
&dn=Standard+Torrent\
&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337";

    let mut magnet = MagnetLink::parse(uri).expect("Parse standard magnet");
    assert!(!magnet.is_webtorrent());
    assert!(!magnet.has_web_seeds());

    // Inject web seeds
    let added_ws = WebTorrentMagnetInjector::inject_web_seeds(
        &mut magnet,
        &["https://cdn.example.org/files/".to_string()],
    );
    assert_eq!(added_ws, 1);
    assert!(magnet.has_web_seeds());

    // Injecting same web seed again should be ignored
    let duplicate_ws = WebTorrentMagnetInjector::inject_web_seeds(
        &mut magnet,
        &["https://cdn.example.org/files/".to_string()],
    );
    assert_eq!(duplicate_ws, 0);

    // Inject WebTorrent default WebSocket trackers
    let added_ws_trackers = WebTorrentMagnetInjector::inject_websocket_trackers(&mut magnet, None);
    assert_eq!(added_ws_trackers, DEFAULT_WEBTORRENT_TRACKERS.len());
    assert!(magnet.is_webtorrent());

    for def in DEFAULT_WEBTORRENT_TRACKERS {
        assert!(magnet.trackers.contains(&def.to_string()));
    }

    // Test direct URI rewriting helper
    let raw_uri = "magnet:?xt=urn:btih:aabbccddeeff00112233aabbccddeeff00112233&dn=Sample";
    let injected_uri = WebTorrentMagnetInjector::inject_into_uri(
        raw_uri,
        &["https://webseed.sample.com/".to_string()],
        true,
    )
    .expect("Inject into URI");

    let parsed_injected = MagnetLink::parse(&injected_uri).expect("Parse rewritten URI");
    assert!(parsed_injected.has_web_seeds());
    assert!(parsed_injected.is_webtorrent());
    assert_eq!(parsed_injected.web_seeds[0], "https://webseed.sample.com/");
    assert_eq!(parsed_injected.websocket_trackers().len(), 3);
}

#[test]
fn test_webtorrent_magnet_injector_context_injection_and_bridge_bootstrap() {
    let mut config = SessionConfig::default();
    config.webtorrent_bridge_mode = true;
    config.enable_webtorrent = true;

    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0xAA; 20],
        vec!["udp://tracker.example.com:6969".to_string()],
        Arc::new(SequentialSelector),
        std::path::Path::new("."),
        config,
    )
    .expect("Create mock magnet context");

    let uri = "magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
&ws=https%3A%2F%2Fmirror1.org%2F\
&ws=https%3A%2F%2Fmirror2.org%2F";
    let magnet = MagnetLink::parse(uri).expect("Parse magnet");

    let summary = WebTorrentMagnetInjector::inject_into_context(&magnet, &mut ctx, true);

    // Verify WebSeeds were injected
    assert_eq!(summary.injected_web_seeds_count, 2);
    assert_eq!(ctx.web_seeds.len(), 2);
    assert_eq!(ctx.web_seeds[0], "https://mirror1.org/");
    assert_eq!(ctx.web_seeds[1], "https://mirror2.org/");

    // Verify WebSocket trackers were injected automatically as fallback
    assert_eq!(summary.injected_trackers_count, 3);
    let ws_trackers: Vec<_> = ctx
        .tracker_urls
        .iter()
        .filter(|t| WebTorrentMagnetInjector::is_websocket_tracker(t))
        .collect();
    assert_eq!(ws_trackers.len(), 3);

    // Verify bridge was bootstrapped with offers
    assert!(summary.bridge_bootstrapped);
}

#[test]
fn test_magnet_uri_builder_with_webtorrent_defaults() {
    let v1_hash = vec![0x12; 20];
    let v2_hash = vec![0x34; 32];

    let magnet = MagnetUriBuilder::new(&v1_hash)
        .with_v2_hash(&v2_hash)
        .with_display_name("Blender Open Movie")
        .with_tracker("udp://tracker.blender.org:6969")
        .with_web_seed("https://download.blender.org/demo/movies/")
        .with_exact_source("https://download.blender.org/demo/movies/open.torrent")
        .with_webtorrent_defaults()
        .build()
        .expect("Build MagnetLink with builder");

    assert!(magnet.is_hybrid());
    assert_eq!(magnet.display_name.as_deref(), Some("Blender Open Movie"));
    assert_eq!(magnet.web_seeds.len(), 1);
    assert_eq!(magnet.exact_sources.len(), 1);
    // 1 custom tracker + 3 default webtorrent trackers = 4
    assert_eq!(magnet.trackers.len(), 4);
    assert_eq!(magnet.websocket_trackers().len(), 3);

    let uri_str = magnet.to_uri();
    assert!(uri_str.contains("xt=urn:btih:"));
    assert!(uri_str.contains("xt=urn:btmh:1220"));
    assert!(uri_str.contains("tr=wss%3A%2F%2Ftracker.openwebtorrent.com"));
    assert!(uri_str.contains("ws=https%3A%2F%2Fdownload.blender.org%2Fdemo%2Fmovies%2F"));
}

#[test]
fn test_session_builder_and_runtime_webseed_websocket_injection() {
    let temp_dir = std::env::temp_dir().join("bt_test_magnet_injector");
    let _ = std::fs::remove_dir_all(&temp_dir);

    let magnet_uri = "magnet:?xt=urn:btih:4455667788990011223344556677889900112233\
&dn=Runtime+Test\
&ws=https%3A%2F%2Fmagnet-seed.org%2F";

    let mut session = TorrentSessionBuilder::from_magnet(magnet_uri, &temp_dir)
        .with_web_seed("https://builder-seed-1.org/")
        .with_web_seeds(vec!["https://builder-seed-2.org/".to_string()])
        .webtorrent_bridge_mode(true)
        .enable_webtorrent(true)
        .build()
        .expect("Build session from magnet with injector");

    // Both magnet web seeds and builder web seeds should be active
    let active_seeds = session.web_seeds();
    assert!(active_seeds.contains(&"https://magnet-seed.org/".to_string()));
    assert!(active_seeds.contains(&"https://builder-seed-1.org/".to_string()));
    assert!(active_seeds.contains(&"https://builder-seed-2.org/".to_string()));

    // Auto-injected default WebTorrent trackers should be active
    let ws_trackers = session.websocket_trackers();
    assert_eq!(ws_trackers.len(), 3);
    for def in DEFAULT_WEBTORRENT_TRACKERS {
        assert!(ws_trackers.contains(&def.to_string()));
    }

    // Dynamic runtime web seed injection
    let added_runtime = session.inject_web_seeds(vec![
        "https://dynamic-seed.org/".to_string(),
        "https://builder-seed-1.org/".to_string(), // duplicate
    ]);
    assert_eq!(added_runtime, 1);
    assert!(
        session
            .web_seeds()
            .contains(&"https://dynamic-seed.org/".to_string())
    );

    // Dynamic runtime websocket tracker injection
    let added_ws = session.inject_websocket_trackers(vec![
        "wss://custom-community-tracker.net".to_string(),
        "udp://not-a-websocket-tracker.com:6969".to_string(), // non-websocket ignored
    ]);
    assert_eq!(added_ws, 1);
    assert!(
        session
            .websocket_trackers()
            .contains(&"wss://custom-community-tracker.net".to_string())
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}
