//! WebTorrent Magnet URI Web-Seed & WebSocket Tracker Injector
//!
//! Provides automated extraction of WebSeeds (`ws=`), exact sources (`xs=`), and WebSocket trackers
//! (`wss://` and `ws://`) from BitTorrent Magnet URIs, along with automated injection of public WebTorrent
//! signaling trackers for instant browser-to-desktop bridging.

use crate::core::torrent_context::TorrentContext;
use crate::error::BitTorrentError;
use crate::magnet::MagnetLink;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use std::sync::{Arc, Mutex};

/// Canonical public WebTorrent WebSocket trackers used for browser-to-desktop cross-swarm bridging.
pub const DEFAULT_WEBTORRENT_TRACKERS: &[&str] = &[
    "wss://tracker.openwebtorrent.com",
    "wss://tracker.btorrent.xyz",
    "wss://tracker.fastcast.nz",
];

/// Detailed report resulting from analyzing and extracting tracker and web seed endpoints from a Magnet URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerExtractionReport {
    /// Discovered WebTorrent WebSocket trackers (`wss://` or `ws://`).
    pub websocket_trackers: Vec<String>,
    /// Discovered standard UDP trackers (`udp://`).
    pub udp_trackers: Vec<String>,
    /// Discovered standard HTTP/HTTPS trackers (`http://` or `https://`).
    pub http_trackers: Vec<String>,
    /// Discovered BEP 19 WebSeed mirror URLs (`ws=`).
    pub web_seeds: Vec<String>,
    /// Discovered BEP 19 exact source URLs (`xs=`).
    pub exact_sources: Vec<String>,
    /// Total count of all trackers discovered.
    pub total_trackers: usize,
    /// Indicates whether at least one WebTorrent WebSocket tracker is present.
    pub has_webtorrent: bool,
}

/// Summary report returned after injecting WebSocket trackers or WebSeeds into a context or session.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct InjectionSummary {
    /// Number of newly injected tracker URLs.
    pub injected_trackers_count: usize,
    /// Number of newly injected WebSeed mirror URLs.
    pub injected_web_seeds_count: usize,
    /// List of newly injected tracker URLs.
    pub injected_trackers: Vec<String>,
    /// List of newly injected WebSeed mirror URLs.
    pub injected_web_seeds: Vec<String>,
    /// Indicates whether the WebTorrent bridge was initialized with initial offers.
    pub bridge_bootstrapped: bool,
}

/// Injector service for automated extraction and bootstrapping of WebSeeds and WebSocket trackers.
pub struct WebTorrentMagnetInjector;

impl WebTorrentMagnetInjector {
    /// Analyzes a parsed [`MagnetLink`] and categorizes all tracker endpoints and WebSeeds.
    pub fn extract_trackers(magnet: &MagnetLink) -> TrackerExtractionReport {
        let websocket_trackers = magnet.websocket_trackers();
        let udp_trackers = magnet.udp_trackers();
        let http_trackers = magnet.http_trackers();
        let web_seeds = magnet.web_seeds.clone();
        let exact_sources = magnet.exact_sources.clone();
        let total_trackers = magnet.trackers.len();
        let has_webtorrent = !websocket_trackers.is_empty();

        TrackerExtractionReport {
            websocket_trackers,
            udp_trackers,
            http_trackers,
            web_seeds,
            exact_sources,
            total_trackers,
            has_webtorrent,
        }
    }

    /// Returns `true` if the specified URL is a WebSocket tracker endpoint (`wss://` or `ws://`).
    pub fn is_websocket_tracker(url: &str) -> bool {
        url.starts_with("wss://") || url.starts_with("ws://")
    }

    /// Injects WebTorrent WebSocket trackers into a [`MagnetLink`].
    ///
    /// If `custom_trackers` is provided, those are used; otherwise [`DEFAULT_WEBTORRENT_TRACKERS`] are used.
    /// Existing trackers are preserved and duplicates are prevented.
    /// Returns the number of newly added trackers.
    pub fn inject_websocket_trackers(
        magnet: &mut MagnetLink,
        custom_trackers: Option<&[&str]>,
    ) -> usize {
        let candidates: &[&str] = custom_trackers.unwrap_or(DEFAULT_WEBTORRENT_TRACKERS);
        let mut added = 0;
        for &t in candidates {
            let s = t.to_string();
            if !magnet.trackers.contains(&s) {
                magnet.trackers.push(s);
                added += 1;
            }
        }
        added
    }

    /// Injects BEP 19 WebSeed mirror URLs into a [`MagnetLink`], preventing duplicates.
    /// Returns the number of newly added WebSeeds.
    pub fn inject_web_seeds(magnet: &mut MagnetLink, web_seeds: &[String]) -> usize {
        let mut added = 0;
        for s in web_seeds {
            if !magnet.web_seeds.contains(s) {
                magnet.web_seeds.push(s.clone());
                added += 1;
            }
        }
        added
    }

    /// Parses a raw magnet URI string, injects optional WebSeeds and/or default WebTorrent trackers,
    /// and formats the updated canonical magnet URI.
    pub fn inject_into_uri(
        uri: &str,
        web_seeds: &[String],
        inject_default_ws_trackers: bool,
    ) -> Result<String, BitTorrentError> {
        let mut magnet = MagnetLink::parse(uri)?;
        if !web_seeds.is_empty() {
            Self::inject_web_seeds(&mut magnet, web_seeds);
        }
        if inject_default_ws_trackers {
            Self::inject_websocket_trackers(&mut magnet, None);
        }
        Ok(magnet.to_uri())
    }

    /// Injects extracted WebSeeds and WebSocket trackers from a [`MagnetLink`] into a [`TorrentContext`].
    ///
    /// If `auto_inject_defaults` is `true` and the context contains no WebSocket trackers,
    /// canonical WebTorrent trackers ([`DEFAULT_WEBTORRENT_TRACKERS`]) will be automatically injected.
    /// If WebTorrent bridge mode is enabled, pre-initializes SDP offers for instant browser connections.
    pub fn inject_into_context(
        magnet: &MagnetLink,
        context: &mut TorrentContext,
        auto_inject_defaults: bool,
    ) -> InjectionSummary {
        let mut summary = InjectionSummary::default();

        // 1. Inject WebSeeds
        for ws in &magnet.web_seeds {
            if !context.web_seeds.contains(ws) {
                context.web_seeds.push(ws.clone());
                summary.injected_web_seeds.push(ws.clone());
            }
        }
        summary.injected_web_seeds_count = summary.injected_web_seeds.len();

        // 2. Inject Trackers from Magnet
        for tr in &magnet.trackers {
            if !context.tracker_urls.contains(tr) {
                context.tracker_urls.push(tr.clone());
                summary.injected_trackers.push(tr.clone());
            }
        }

        // 3. Fallback WebTorrent Injection if enabled and no WebSocket tracker present
        let has_ws = context
            .tracker_urls
            .iter()
            .any(|u| Self::is_websocket_tracker(u));
        if auto_inject_defaults && !has_ws {
            for &def_tr in DEFAULT_WEBTORRENT_TRACKERS {
                let s = def_tr.to_string();
                if !context.tracker_urls.contains(&s) {
                    context.tracker_urls.push(s.clone());
                    summary.injected_trackers.push(s);
                }
            }
        }
        summary.injected_trackers_count = summary.injected_trackers.len();

        // 4. Ensure primary tracker_url is set if empty
        if context.tracker_url.is_empty() {
            if let Some(first) = context.tracker_urls.first() {
                context.tracker_url = first.clone();
            }
        }

        // 5. Pre-generate WebTorrent bridge offers if bridge mode is active
        #[cfg(feature = "webrtc")]
        {
            if context.config.webtorrent_bridge_mode {
                let offers = context.webrtc_bridge.generate_offers(4);
                if !offers.is_empty() {
                    summary.bridge_bootstrapped = true;
                }
            }
        }

        summary
    }

    /// Injects WebSeeds and WebSocket trackers into a thread-safe mutex-guarded [`TorrentContext`].
    pub fn inject_into_shared_context(
        magnet: &MagnetLink,
        context: &Mutex<TorrentContext>,
        auto_inject_defaults: bool,
    ) -> InjectionSummary {
        let mut guard = context.lock().unwrap();
        Self::inject_into_context(magnet, &mut guard, auto_inject_defaults)
    }

    /// Bootstraps the WebTorrent bridge coordinator for an active session by generating
    /// SDP offers and returning the number of generated offers.
    #[cfg(feature = "webrtc")]
    pub fn bootstrap_bridge(
        context: Arc<Mutex<TorrentContext>>,
        num_offers: usize,
    ) -> Result<usize, BitTorrentError> {
        let guard = context.lock().unwrap();
        let offers = guard.webrtc_bridge.generate_offers(num_offers);
        Ok(offers.len())
    }
}
