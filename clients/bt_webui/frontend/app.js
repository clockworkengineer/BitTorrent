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
                <td><strong>${t.name || 'Resolving...'}</strong></td>
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
                <td><small>${t.download_dir}</small></td>
                <td>
                    <div class="actions-cell">
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

// Init
connectWebSocket();
