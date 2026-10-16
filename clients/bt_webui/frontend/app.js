let ws;
let toastTimeout;
const torrentsBody = document.getElementById('torrents-body');
const magnetInput = document.getElementById('magnet-input');
const dirInput = document.getElementById('dir-input');
const addBtn = document.getElementById('add-btn');
const statusBadge = document.getElementById('daemon-status-badge');

function showToast(message, isError = false) {
    const toast = document.getElementById('toast');
    toast.textContent = message;
    toast.style.borderColor = isError ? 'rgba(239, 68, 68, 0.4)' : 'rgba(16, 185, 129, 0.4)';
    toast.classList.remove('hidden');
    
    if (toastTimeout) {
        clearTimeout(toastTimeout);
    }
    
    toastTimeout = setTimeout(() => {
        toast.classList.add('hidden');
    }, 4000);
}

function connectWebSocket() {
    const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    ws = new WebSocket(`${proto}//${window.location.host}/api/ws`);

    ws.onopen = () => {
        statusBadge.innerHTML = '<span class="indicator"></span> Connected';
        statusBadge.style.color = '#10b981';
        statusBadge.style.background = 'rgba(16, 185, 129, 0.1)';
        statusBadge.style.borderColor = 'rgba(16, 185, 129, 0.2)';
    };

    ws.onmessage = (event) => {
        try {
            const data = JSON.parse(event.data);
            if (data.torrents) {
                renderTorrents(data.torrents);
            } else if (data.error) {
                torrentsBody.innerHTML = `
                    <tr class="empty-row">
                        <td colspan="8" style="color: var(--danger-hover)">Error: ${data.error} (Is the daemon running?)</td>
                    </tr>
                `;
            }
        } catch (e) {
            console.error("Error parsing status message:", e);
        }
    };

    ws.onclose = () => {
        statusBadge.innerHTML = '<span class="indicator" style="background:#ef4444;box-shadow:0 0 8px #ef4444"></span> Reconnecting...';
        statusBadge.style.color = '#ef4444';
        statusBadge.style.background = 'rgba(239, 68, 68, 0.1)';
        statusBadge.style.borderColor = 'rgba(239, 68, 68, 0.2)';
        setTimeout(connectWebSocket, 3000);
    };
}

function renderTorrents(torrents) {
    if (torrents.length === 0) {
        torrentsBody.innerHTML = `
            <tr class="empty-row">
                <td colspan="8">No active torrents in daemon.</td>
            </tr>
        `;
        return;
    }

    let html = '';
    torrents.forEach((t) => {
        const progressPercent = (t.progress * 100).toFixed(1);
        const speedText = fmtBytes(t.download_rate) + '/s';
        const sizeText = fmtBytes(t.downloaded) + ' / ' + fmtBytes(t.total_size);
        const statusClass = `status-${t.status.toLowerCase()}`;
        const peersText = `${t.peers_active} (${t.peers_connected})`;

        // Determine which action button to show
        const isPaused = t.status.toLowerCase() === 'paused';
        const actionBtn = isPaused 
            ? `<button class="secondary-btn" onclick="resumeTorrent('${t.info_hash}')">Resume</button>`
            : `<button class="secondary-btn" onclick="pauseTorrent('${t.info_hash}')">Pause</button>`;

        html += `
            <tr>
                <td class="torrent-name" title="${t.name || 'Resolving...'}"><strong>${t.name || 'Resolving...'}</strong></td>
                <td>
                    <div class="progress-container">
                        <div class="progress-track">
                            <div class="progress-fill" style="width: ${progressPercent}%"></div>
                        </div>
                        <span class="progress-text">${progressPercent}%</span>
                    </div>
                </td>
                <td><span class="status-pill ${statusClass}">${t.status}</span></td>
                <td>${peersText}</td>
                <td>${speedText}</td>
                <td>${sizeText}</td>
                <td class="dir-cell" title="${t.download_dir}"><small>${t.download_dir}</small></td>
                <td>
                    <div class="actions-cell">
                        <button class="stream-btn" onclick="openStreamModal('${t.info_hash}', '${escapeHtml(t.name || 'Torrent Stream')}')">▶ Stream</button>
                        <button class="secondary-btn" onclick="openSwarmModal('${t.info_hash}', '${escapeHtml(t.name || 'Torrent Swarm')}')">👥 Swarm</button>
                        ${actionBtn}
                        <button class="danger-btn" onclick="removeTorrent('${t.info_hash}')">Remove</button>
                    </div>
                </td>
            </tr>
        `;
    });
    torrentsBody.innerHTML = html;
}

function fmtBytes(bytes) {
    if (bytes >= 1073741824) return (bytes / 1073741824).toFixed(2) + ' GB';
    if (bytes >= 1048576) return (bytes / 1048576).toFixed(1) + ' MB';
    if (bytes >= 1024) return (bytes / 1024).toFixed(1) + ' KB';
    return bytes + ' B';
}

// Actions
window.pauseTorrent = async (hash) => {
    try {
        const res = await fetch(`/api/torrents/${hash}/pause`, { method: 'POST' });
        const data = await res.json();
        if (res.ok) showToast("Torrent paused successfully");
        else showToast(data.reason || "Failed to pause torrent", true);
    } catch (e) {
        showToast("Error pausing torrent", true);
    }
};

window.resumeTorrent = async (hash) => {
    try {
        const res = await fetch(`/api/torrents/${hash}/resume`, { method: 'POST' });
        const data = await res.json();
        if (res.ok) showToast("Torrent resumed successfully");
        else showToast(data.reason || "Failed to resume torrent", true);
    } catch (e) {
        showToast("Error resuming torrent", true);
    }
};

window.removeTorrent = async (hash) => {
    const purge = confirm("Do you also want to delete the downloaded files from the disk?");
    try {
        const res = await fetch(`/api/torrents/${hash}?purge=${purge}`, { method: 'DELETE' });
        const data = await res.json();
        if (res.ok) showToast("Torrent removed successfully");
        else showToast(data.reason || "Failed to remove torrent", true);
    } catch (e) {
        showToast("Error removing torrent", true);
    }
};

addBtn.addEventListener('click', async () => {
    const torrent_path = magnetInput.value.trim();
    const download_dir = dirInput.value.trim() || null;
    if (!torrent_path) {
        showToast("Please enter a torrent file path or magnet link", true);
        return;
    }

    try {
        const res = await fetch('/api/torrents/add', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ torrent_path, download_dir })
        });
        const data = await res.json();
        if (res.ok) {
            showToast("Torrent added successfully");
            magnetInput.value = '';
            dirInput.value = '';
        } else {
            showToast(data.reason || "Failed to add torrent", true);
        }
    } catch (e) {
        showToast("Error adding torrent", true);
    }
});

function escapeHtml(str) {
    if (!str) return '';
    return str.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#039;");
}

// Media Streaming Modal
const streamModal = document.getElementById('stream-modal');
const streamPlayer = document.getElementById('stream-player');
const streamTitle = document.getElementById('stream-title');
const streamFileSelect = document.getElementById('stream-file-select');
const streamStatus = document.getElementById('stream-status');
const streamUrlDisplay = document.getElementById('stream-url-display');
const streamModalClose = document.getElementById('stream-modal-close');

let currentStreamHash = null;

window.openStreamModal = async (infoHash, torrentName) => {
    currentStreamHash = infoHash;
    streamTitle.textContent = torrentName || 'Media Stream';
    streamStatus.textContent = 'Probing torrent media files...';
    streamStatus.style.color = '#38bdf8';
    
    // Clear previous state
    streamFileSelect.innerHTML = '';
    streamFileSelect.classList.add('hidden');
    
    // Show modal
    streamModal.classList.remove('hidden');

    try {
        const res = await fetch(`/api/torrents/${infoHash}/files`);
        if (res.ok) {
            const data = await res.json();
            if (data.files && data.files.length > 0) {
                const mediaFiles = data.files.filter(f => f.is_media);
                const filesToShow = mediaFiles.length > 0 ? mediaFiles : data.files;

                if (filesToShow.length > 1) {
                    filesToShow.forEach((f) => {
                        const opt = document.createElement('option');
                        opt.value = f.index;
                        opt.textContent = `${f.name} (${fmtBytes(f.length)})`;
                        streamFileSelect.appendChild(opt);
                    });
                    streamFileSelect.classList.remove('hidden');
                    streamFileSelect.onchange = () => {
                        loadStreamSource(infoHash, streamFileSelect.value);
                    };
                }

                // Default to first media file
                const defaultIndex = filesToShow[0].index;
                loadStreamSource(infoHash, defaultIndex);
                return;
            }
        }
    } catch (e) {
        console.warn("Could not query files list; falling back to direct stream:", e);
    }

    // Fallback: direct torrent stream
    loadStreamSource(infoHash);
};

function loadStreamSource(infoHash, fileIndex) {
    const streamEndpoint = fileIndex !== undefined && fileIndex !== null 
        ? `/api/stream/${infoHash}/${fileIndex}`
        : `/api/stream/${infoHash}`;

    streamUrlDisplay.textContent = streamEndpoint;
    streamStatus.textContent = 'Buffering streaming pieces from peers...';
    streamStatus.style.color = '#38bdf8';

    streamPlayer.pause();
    streamPlayer.src = streamEndpoint;
    streamPlayer.load();
    streamPlayer.play().catch(e => {
        // Autoplay may be restricted by browser until user clicks play
        console.log("Autoplay deferred:", e);
    });
}

streamPlayer.addEventListener('waiting', () => {
    streamStatus.textContent = 'Awaiting next piece blocks from swarm...';
    streamStatus.style.color = '#fbbf24';
});

streamPlayer.addEventListener('playing', () => {
    streamStatus.textContent = 'Streaming live (seeking enabled)';
    streamStatus.style.color = '#34d399';
});

streamPlayer.addEventListener('canplay', () => {
    streamStatus.textContent = 'Buffer ready for playback';
    streamStatus.style.color = '#34d399';
});

streamPlayer.addEventListener('error', () => {
    streamStatus.textContent = 'Streaming error (ensure daemon is active & peers connected)';
    streamStatus.style.color = '#f87171';
});

window.closeStreamModal = () => {
    streamPlayer.pause();
    streamPlayer.removeAttribute('src');
    streamPlayer.load();
    streamModal.classList.add('hidden');
    currentStreamHash = null;
};

streamModalClose.addEventListener('click', closeStreamModal);

// Close on backdrop click
streamModal.addEventListener('click', (e) => {
    if (e.target === streamModal) {
        closeStreamModal();
    }
});

// Swarm & Piece Visualizer Controller
const swarmModal = document.getElementById('swarm-modal');
const swarmModalClose = document.getElementById('swarm-modal-close');
const swarmTitle = document.getElementById('swarm-title');
const swarmPeerCount = document.getElementById('swarm-peer-count');
const swarmTotalPeers = document.getElementById('swarm-total-peers');
const swarmTotalDown = document.getElementById('swarm-total-down');
const swarmTotalUp = document.getElementById('swarm-total-up');
const swarmPeersBody = document.getElementById('swarm-peers-body');
const swarmPeersPanel = document.getElementById('swarm-peers-panel');
const swarmPiecesPanel = document.getElementById('swarm-pieces-panel');
const tabPeersBtn = document.getElementById('tab-peers-btn');
const tabPiecesBtn = document.getElementById('tab-pieces-btn');
const piecesCompletionText = document.getElementById('pieces-completion-text');
const piecesSizeText = document.getElementById('pieces-size-text');
const piecesRarityText = document.getElementById('pieces-rarity-text');
const piecesGrid = document.getElementById('pieces-grid');
const swarmHashDisplay = document.getElementById('swarm-hash-display');
const visualizerHint = document.getElementById('visualizer-hint');

let currentSwarmHash = null;
let swarmPollInterval = null;
let activeSwarmTab = 'peers';

window.switchSwarmTab = (tab) => {
    activeSwarmTab = tab;
    if (tab === 'peers') {
        tabPeersBtn.classList.add('active');
        tabPiecesBtn.classList.remove('active');
        swarmPeersPanel.classList.remove('hidden');
        swarmPiecesPanel.classList.add('hidden');
        visualizerHint.textContent = 'Live peer connections and transfer speeds';
    } else {
        tabPeersBtn.classList.remove('active');
        tabPiecesBtn.classList.add('active');
        swarmPeersPanel.classList.add('hidden');
        swarmPiecesPanel.classList.remove('hidden');
        visualizerHint.textContent = 'Hover over piece tiles for byte offsets and availability';
    }
};

window.openSwarmModal = (infoHash, torrentName) => {
    currentSwarmHash = infoHash;
    swarmTitle.textContent = `${torrentName} — Swarm & Pieces`;
    swarmHashDisplay.textContent = infoHash;
    swarmModal.classList.remove('hidden');

    // Initial fetch
    fetchSwarmData();

    // Poll every 1500 ms while modal is visible
    if (swarmPollInterval) clearInterval(swarmPollInterval);
    swarmPollInterval = setInterval(fetchSwarmData, 1500);
};

window.closeSwarmModal = () => {
    swarmModal.classList.add('hidden');
    currentSwarmHash = null;
    if (swarmPollInterval) {
        clearInterval(swarmPollInterval);
        swarmPollInterval = null;
    }
};

swarmModalClose.addEventListener('click', closeSwarmModal);
swarmModal.addEventListener('click', (e) => {
    if (e.target === swarmModal) closeSwarmModal();
});

document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !swarmModal.classList.contains('hidden')) {
        closeSwarmModal();
    }
});

async function fetchSwarmData() {
    if (!currentSwarmHash) return;

    try {
        // Fetch peers and pieces concurrently
        const [peersRes, piecesRes] = await Promise.all([
            fetch(`/api/torrents/${currentSwarmHash}/peers`),
            fetch(`/api/torrents/${currentSwarmHash}/pieces`),
        ]);

        if (peersRes.ok) {
            const peersData = await peersRes.json();
            renderSwarmPeers(peersData.peers || []);
        }

        if (piecesRes.ok) {
            const piecesData = await piecesRes.json();
            if (piecesData.pieces) {
                renderPieceHeatmap(piecesData.pieces);
            }
        }
    } catch (e) {
        console.warn("Could not poll swarm data:", e);
    }
}

function renderSwarmPeers(peers) {
    swarmPeerCount.textContent = peers.length;
    swarmTotalPeers.textContent = peers.length;

    const totalDown = peers.reduce((sum, p) => sum + (p.down_rate || 0), 0);
    const totalUp = peers.reduce((sum, p) => sum + (p.up_rate || 0), 0);

    swarmTotalDown.textContent = fmtBytes(totalDown) + '/s';
    swarmTotalUp.textContent = fmtBytes(totalUp) + '/s';

    if (peers.length === 0) {
        swarmPeersBody.innerHTML = '<tr class="empty-row"><td colspan="6">No peers currently connected.</td></tr>';
        return;
    }

    let rowsHtml = '';
    peers.forEach((p) => {
        const progressPct = ((p.progress || 0) * 100).toFixed(1);
        const downStr = p.down_rate > 0 ? `<span style="color:#10b981;font-weight:600">${fmtBytes(p.down_rate)}/s</span>` : '-';
        const upStr = p.up_rate > 0 ? `<span style="color:#38bdf8;font-weight:600">${fmtBytes(p.up_rate)}/s</span>` : '-';

        rowsHtml += `
            <tr>
                <td><strong>${escapeHtml(p.client || 'Unknown')}</strong></td>
                <td class="code-font">${escapeHtml(p.ip)}:${p.port}</td>
                <td>
                    <div class="progress-container">
                        <div class="progress-track" style="width:70px">
                            <div class="progress-fill" style="width: ${progressPct}%"></div>
                        </div>
                        <span class="progress-text">${progressPct}%</span>
                    </div>
                </td>
                <td>${downStr}</td>
                <td>${upStr}</td>
                <td title="I/i: Interested, C/c: Choking, H/h: Peer Interested, U/u: Peer Unchoked, E: Extension protocol, F: Fast extension"><span class="code-font">${p.flags}</span></td>
            </tr>
        `;
    });
    swarmPeersBody.innerHTML = rowsHtml;
}

function renderPieceHeatmap(info) {
    const total = info.total_pieces || 0;
    const have = info.have_pieces || 0;
    const pct = total > 0 ? ((have / total) * 100).toFixed(1) : '0.0';

    piecesCompletionText.textContent = `${have} / ${total} (${pct}%)`;
    piecesSizeText.textContent = fmtBytes(info.piece_length || 0);

    const maxRarity = info.swarm_rarity ? Math.max(0, ...info.swarm_rarity) : 0;
    piecesRarityText.textContent = `${maxRarity} peer copies`;

    // Only rebuild DOM tiles if piece count changed or on first render
    const currentTiles = piecesGrid.children.length;
    if (currentTiles !== total) {
        piecesGrid.innerHTML = '';
        const frag = document.createDocumentFragment();
        for (let i = 0; i < total; i++) {
            const tile = document.createElement('div');
            tile.className = 'piece-tile';
            tile.dataset.index = i;
            frag.appendChild(tile);
        }
        piecesGrid.appendChild(frag);
    }

    // Update classes and titles
    const tiles = piecesGrid.children;
    for (let i = 0; i < total; i++) {
        const tile = tiles[i];
        const isHave = info.local_have && info.local_have[i];
        const rarity = (info.swarm_rarity && info.swarm_rarity[i]) || 0;
        const offset = fmtBytes(i * (info.piece_length || 0));

        let statusText = 'Missing';
        let cls = 'piece-tile ';

        if (isHave) {
            cls += 'piece-downloaded';
            statusText = 'Downloaded & Verified';
        } else if (rarity > 0) {
            if (rarity === 1) {
                cls += 'piece-rare';
                statusText = 'Rare (1 peer copy)';
            } else if (rarity <= 3) {
                cls += 'piece-swarm-low';
                statusText = `In Swarm (${rarity} peer copies)`;
            } else {
                cls += 'piece-swarm-high';
                statusText = `Well-seeded (${rarity} peer copies)`;
            }
        } else {
            cls += 'piece-missing';
        }

        tile.className = cls;
        tile.title = `Piece #${i}\nOffset: ${offset}\nStatus: ${statusText}`;
    }
}

// Init
connectWebSocket();


