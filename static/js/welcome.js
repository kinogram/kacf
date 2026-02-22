(() => {
    const README_URL = 'https://raw.githubusercontent.com/kinogram/kacf/main/README.md';

    function escapeHtml(text) {
        return String(text || '')
            .replace(/&/g, '&amp;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;');
    }

    function markdownToHtml(md) {
        const src = String(md || '');
        const lines = src.split(/\r?\n/).slice(0, 220);
        const out = [];
        let inList = false;
        let inCode = false;
        for (const raw of lines) {
            const line = raw || '';
            if (line.trim().startsWith('```')) {
                if (!inCode) {
                    if (inList) {
                        out.push('</ul>');
                        inList = false;
                    }
                    out.push('<pre><code>');
                    inCode = true;
                } else {
                    out.push('</code></pre>');
                    inCode = false;
                }
                continue;
            }
            if (inCode) {
                out.push(`${escapeHtml(line)}\n`);
                continue;
            }
            if (!line.trim()) {
                if (inList) {
                    out.push('</ul>');
                    inList = false;
                }
                continue;
            }
            if (line.startsWith('# ')) {
                if (inList) {
                    out.push('</ul>');
                    inList = false;
                }
                out.push(`<h3>${escapeHtml(line.slice(2).trim())}</h3>`);
                continue;
            }
            if (line.startsWith('## ')) {
                if (inList) {
                    out.push('</ul>');
                    inList = false;
                }
                out.push(`<h4>${escapeHtml(line.slice(3).trim())}</h4>`);
                continue;
            }
            if (line.startsWith('- ') || line.startsWith('* ')) {
                if (!inList) {
                    out.push('<ul>');
                    inList = true;
                }
                out.push(`<li>${escapeHtml(line.slice(2).trim())}</li>`);
                continue;
            }
            if (inList) {
                out.push('</ul>');
                inList = false;
            }
            out.push(`<p>${escapeHtml(line.trim())}</p>`);
        }
        if (inList) out.push('</ul>');
        if (inCode) out.push('</code></pre>');
        return out.join('');
    }

    async function refreshEnterButton() {
        const btn = document.getElementById('enter_program_btn');
        if (!btn) return;
        try {
            const resp = await fetch('/auth/me', { cache: 'no-store' });
            if (!resp.ok) return;
            const data = await resp.json();
            if (data && data.logged_in) {
                btn.href = '/';
                btn.textContent = '进入工作台';
            }
        } catch (_e) {}
    }

    async function loadReadme() {
        const root = document.getElementById('welcome_readme');
        if (!root) return;
        try {
            const resp = await fetch(README_URL, { cache: 'no-store' });
            if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
            const text = await resp.text();
            root.innerHTML = markdownToHtml(text);
        } catch (_e) {
            root.innerHTML = '<p>加载失败，请直接打开 GitHub 仓库查看完整介绍。</p>';
        }
    }

    (async () => {
        await Promise.all([refreshEnterButton(), loadReadme()]);
    })();
})();
