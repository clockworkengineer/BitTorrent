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

// Close on Escape key
document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !streamModal.classList.contains('hidden')) {
        closeStreamModal();
    }
});

// Init
connectWebSocket();

