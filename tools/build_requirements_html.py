#!/usr/bin/env python3
"""Build requirements.html from final-requirements.md + diagrams/*.svg (self-contained)."""
import base64, html, os, re, sys

import os
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MD = f"{ROOT}/research/final-requirements.md"
DIAG = f"{ROOT}/research/diagrams"
SHEET = f"{ROOT}/mog-rive/shots/sheet.png"
OUT = f"{ROOT}/requirements.html"
TITLE = "Mother Databricks: Bottle (formerly Mother Bricks) & MOG Requirements (ASD-STE100)"

# ---------------- SVGs ----------------
def load_svgs():
    svgs = {}
    for fn in sorted(os.listdir(DIAG)):
        if not fn.endswith(".svg"):
            continue
        slug = re.sub(r"^\d{2}-", "", fn[:-4])
        s = open(os.path.join(DIAG, fn), encoding="utf-8").read()
        s = re.sub(r"<\?xml[^>]*\?>\s*", "", s)
        s = re.sub(r"<!DOCTYPE[^>]*>\s*", "", s)
        pre = f"svg-{slug}"
        ids = re.findall(r'\sid="([^"]+)"', s)
        for i in set(ids):
            s = re.sub(r'(\sid=")' + re.escape(i) + '"', r'\1' + f'{pre}-{i}"', s)
            s = s.replace(f"url(#{i})", f"url(#{pre}-{i})")
            s = s.replace(f'href="#{i}"', f'href="#{pre}-{i}"')
        def fix_aria(m):
            return m.group(1) + " ".join(f"{pre}-{t}" if t in ids else t for t in m.group(2).split()) + '"'
        s = re.sub(r'(aria-labelledby=")([^"]*)"', fix_aria, s)
        # scope the <style> rules to this SVG only
        def scope(m):
            css = m.group(2)
            def rule(r):
                sels = ",".join(f"#{pre} {x.strip()}" for x in r.group(1).split(","))
                return sels + "{" + r.group(2) + "}"
            css = re.sub(r"([^{}]+)\{([^{}]*)\}", lambda r: rule(r) if r.group(1).strip() else r.group(0), css)
            return m.group(1) + css + m.group(3)
        s = re.sub(r"(<style[^>]*>\s*(?:<!\[CDATA\[)?)(.*?)((?:\]\]>)?\s*</style>)", scope, s, flags=re.S)
        s = re.sub(r"<svg\b", f'<svg id="{pre}"', s, count=1)
        t = re.search(r"<title[^>]*>(.*?)</title>", s, re.S)
        svgs[slug] = (s.strip(), html.unescape(t.group(1)) if t else slug)
    return svgs

# ---------------- inline markdown ----------------
PHASES = {"MVP-D": "mvpd", "MVP-M": "mvpm", "v1": "v1", "v2": "v2", "MVP": "mvpd"}
def inline(t):
    parts = re.split(r"(`[^`]+`)", t)
    out = []
    for p in parts:
        if len(p) >= 2 and p.startswith("`") and p.endswith("`"):
            out.append("<code>" + html.escape(p[1:-1].replace("\\|", "|"), quote=False) + "</code>")
            continue
        p = html.escape(p.replace("\\|", "|"), quote=False)
        out.append(p)
    codes = [o for o in out if o.startswith("<code>")]
    s = "".join(o if not o.startswith("<code>") else f"\x00{codes.index(o)}\x00" for o in out)
    # bold / italic (not inside tags: code content has no * conversions issue minimal)
    s = re.sub(r"\*\*(?=\S)(.+?)(?<=\S)\*\*", r"<strong>\1</strong>", s)
    s = re.sub(r"(?<![\w*])\*(?=[^\s*])(.+?)(?<=[^\s*])\*(?![\w*])", r"<em>\1</em>", s)
    # citation autolinks (bare URLs in prose, never inside <code>)
    segs = re.split(r"(\x00\d+\x00)", s)
    for i, g in enumerate(segs):
        if g.startswith("\x00"):
            continue
        def link(m):
            u = m.group(0)
            tail = ""
            while u and u[-1] in ".,;:)":
                tail = u[-1] + tail; u = u[:-1]
            return f'<a class="cite" href="{u}" rel="noopener noreferrer">{u}</a>{tail}'
        segs[i] = re.sub(r"https?://[A-Za-z0-9][^\s<&\"]*", link, g)
    s = "".join(segs)
    s = re.sub(r"\x00(\d+)\x00", lambda m: codes[int(m.group(1))], s)
    # badges
    s = re.sub(r"<strong>(?:<code>)?CONFIRM WITH USER(?:</code>)?:?</strong>:?|<code>CONFIRM WITH USER</code>|(?<![>\w])CONFIRM WITH USER(?![<\w])",
               '<span class="badge confirm" title="Decision made; the user must confirm it">CONFIRM WITH USER</span>', s)
    s = s.replace("[U]", '<span class="badge unv" title="Not verified; a spike closes it">[U]</span>')
    return s

def slugify(t, used):
    b = re.sub(r"[^a-z0-9]+", "-", re.sub(r"<[^>]+>", "", t).lower()).strip("-")[:60] or "s"
    s, n = b, 2
    while s in used:
        s = f"{b}-{n}"; n += 1
    used.add(s)
    return s

# ---------------- cell decorators ----------------
PRI = {"M": ("must", "MUST"), "S": ("should", "SHOULD"), "C": ("could", "COULD")}
SEV = {"Crit": "crit", "High": "high", "Med": "med", "Low": "low"}
def decorate(col, raw, cell_html):
    c = col.strip().lower()
    r = raw.strip()
    if c in ("pri", "priority") and r in PRI:
        k, w = PRI[r]
        return f'<span class="badge pri {k}" title="Pri {r}">{w}</span>'
    if c == "phase" or c == "phases":
        def chip(m):
            v = m.group(0)
            return f'<span class="chip ph-{PHASES[v]}">{v}</span>'
        return re.sub(r"\bMVP-D\b|\bMVP-M\b|(?<![\w-])v[12]\b", chip, cell_html)
    if c in ("sev", "severity") and r in SEV:
        return f'<span class="sev sev-{SEV[r]}">{r}</span>'
    if c == "uc":
        return re.sub(r"\b(UC[123]|MTG|ALL)\b", r'<span class="chip uc">\1</span>', cell_html)
    if c == "status" and re.search(r"\b(UC|RES|AR|SPIKE)\b", r):
        lab = {"UC": "UC: confirm", "RES": "RES", "AR": "AR", "SPIKE": "SPIKE"}
        return re.sub(r"(?<![\w-])(UC|RES|AR|SPIKE)(?![\w-])", lambda m: f'<span class="st st-{m.group(1).lower()}">{lab[m.group(1)]}</span>', cell_html)
    return cell_html

def split_row(line):
    line = line.strip()
    if line.startswith("|"): line = line[1:]
    if line.endswith("|") and not line.endswith("\\|"): line = line[:-1]
    cells, cur, i = [], "", 0
    while i < len(line):
        ch = line[i]
        if ch == "\\" and i + 1 < len(line) and line[i + 1] == "|":
            cur += "\\|"; i += 2; continue
        if ch == "|":
            cells.append(cur); cur = ""
        else:
            cur += ch
        i += 1
    cells.append(cur)
    return [x.strip() for x in cells]

# ---------------- block parser ----------------
def convert(md_lines, svgs):
    out, toc, used_ids, used_slugs = [], [], set(), set()
    confirm_items, uc_cols = [], []
    cur_h2 = None
    i, n = 0, len(md_lines)
    subtitle = ""
    def para_class(txt):
        if "Existential questions" in txt: return ' class="callout existential"'
        if "Questions for user confirmation" in txt: return ' class="callout confirm-box"'
        if "CONFIRM WITH USER" in txt: return ' class="has-confirm"'
        return ""
    while i < n:
        line = md_lines[i]
        st = line.strip()
        if not st:
            i += 1; continue
        if st.startswith("```"):
            lang = st[3:].strip()
            j = i + 1; buf = []
            while j < n and not md_lines[j].strip().startswith("```"):
                buf.append(md_lines[j]); j += 1
            lab = f' data-lang="{html.escape(lang)}"' if lang else ""
            out.append(f'<pre class="code"{lab}><code>' + html.escape("\n".join(buf), quote=False) + "</code></pre>")
            i = j + 1; continue
        if st in ("<details>", "</details>"):
            out.append(st); i += 1; continue
        m = re.match(r"<summary>(.*)</summary>$", st)
        if m:
            out.append(f"<summary>{inline(m.group(1))}</summary>"); i += 1; continue
        m = re.match(r"^(#{1,6})\s+(.*)$", st)
        if m:
            lvl, txt = len(m.group(1)), m.group(2)
            if lvl == 1:
                subtitle = txt; i += 1; continue
            # leftover diagrams go to the end of the Architecture section
            if lvl == 2 and cur_h2 and cur_h2.startswith("4."):
                for slug in [s for s in svgs if s not in placed and s not in referenced]:
                    out.append(figure(slug, svgs, None)); placed.add(slug)
            hid = slugify(txt, used_slugs)
            if lvl == 2:
                cur_h2 = txt
                toc.append((txt, hid, []))
            elif lvl == 3 and toc:
                toc[-1][2].append((txt, hid))
            out.append(f'<h{lvl} id="{hid}">{inline(txt)} <a class="anchor" href="#{hid}" aria-label="Link to this section">#</a></h{lvl}>')
            if lvl == 2 and txt.startswith("4."):
                out.append(diagram_index(md_lines, svgs))
            i += 1; continue
        if re.match(r"^-{3,}$", st):
            out.append("<hr>"); i += 1; continue
        m = re.match(r"^\[\[DIAGRAM:\s*([a-z0-9-]+)\s*\]\]$", st)
        if m:
            slug = m.group(1)
            # absorb the following italic description paragraph into the caption
            j = i + 1
            while j < n and not md_lines[j].strip(): j += 1
            desc = None
            if j < n and re.match(r"^\*[^*].*\*$", md_lines[j].strip()):
                desc = md_lines[j].strip()[1:-1]; i = j
            if slug in svgs:
                out.append(figure(slug, svgs, desc)); placed.add(slug)
            else:
                missing.append(slug)
                out.append(f'<p class="callout warn">Missing diagram: <code>{html.escape(slug)}</code></p>')
                if desc: out.append(f"<p><em>{inline(desc)}</em></p>")
            if slug == "mog-state-machine":
                pending_mascot[0] = True
            i += 1; continue
        if st.startswith("|"):
            rows = []
            while i < n and md_lines[i].strip().startswith("|"):
                rows.append(split_row(md_lines[i])); i += 1
            hdr = rows[0]
            body = rows[2:] if len(rows) > 1 and all(re.match(r"^:?-+:?$", c) for c in rows[1]) else rows[1:]
            t = ['<div class="tw"><table>', "<thead><tr>" + "".join(f'<th scope="col">{inline(h)}</th>' for h in hdr) + "</tr></thead><tbody>"]
            for r in body:
                rawrow = " ".join(r)
                cls = []
                if "CONFIRM WITH USER" in rawrow: cls.append("confirm-row")
                if "Existential" in rawrow or "existential" in rawrow: cls.append("exist-row")
                rid = ""
                key = re.sub(r"[*`]", "", r[0]).strip()
                if re.match(r"^[A-Za-z][A-Za-z0-9.\-]{0,20}$", key):
                    cand = "r-" + key
                    if cand not in used_ids:
                        used_ids.add(cand); rid = f' id="{cand}"'
                if "CONFIRM WITH USER" in rawrow:
                    topic = re.sub(r"[*`]", "", r[1]) if len(r) > 1 else ""
                    confirm_items.append((key, topic, rid[5:-1] if rid else "", cur_h2))
                stc = [k for k, h in enumerate(hdr) if h.strip().lower() == "status"]
                if stc and len(r) > stc[0] and re.search(r"(?<![\w-])UC(?![\w-])", r[stc[0]]) and key.startswith("COL"):
                    uc_cols.append((key, re.sub(r"[*`]", "", r[1]), rid[5:-1] if rid else "", r[stc[0]].strip()))
                tds = []
                for k, c in enumerate(r):
                    col = hdr[k] if k < len(hdr) else ""
                    ch = decorate(col, c, inline(c))
                    tag = "th scope=\"row\"" if k == 0 else "td"
                    tds.append(f"<{tag}>{ch}</{tag.split()[0]}>")
                ca = f' class="{" ".join(cls)}"' if cls else ""
                t.append(f"<tr{rid}{ca}>" + "".join(tds) + "</tr>")
            t.append("</tbody></table></div>")
            out.append("\n".join(t))
            if pending_mascot[0]:
                out.append(MASCOT_HTML); pending_mascot[0] = False
            continue
        if st.startswith(">"):
            buf = []
            while i < n and md_lines[i].strip().startswith(">"):
                buf.append(re.sub(r"^\s*>\s?", "", md_lines[i])); i += 1
            inner, _, _, _, _ = convert_inner(buf, svgs)
            out.append('<blockquote class="callout">' + inner + "</blockquote>")
            continue
        if re.match(r"^([-*]|\d+\.)\s", st):
            ordered = bool(re.match(r"^\d+\.", st))
            tag = "ol" if ordered else "ul"
            start = re.match(r"^(\d+)\.", st)
            sa = f' start="{start.group(1)}"' if start and start.group(1) != "1" else ""
            items = []
            while i < n and re.match(r"^([-*]|\d+\.)\s", md_lines[i].strip()) and bool(re.match(r"^\d+\.", md_lines[i].strip())) == ordered:
                txt = re.sub(r"^([-*]|\d+\.)\s+", "", md_lines[i].strip())
                c = ' class="has-confirm"' if "CONFIRM WITH USER" in txt else ""
                items.append(f"<li{c}>{inline(txt)}</li>"); i += 1
            out.append(f"<{tag}{sa}>" + "".join(items) + f"</{tag}>")
            continue
        buf = []
        while i < n and md_lines[i].strip() and not re.match(r"^(```|#{1,6}\s|\||>|[-*]\s|\d+\.\s|\[\[DIAGRAM|-{3,}$|</?details>|<summary>)", md_lines[i].strip()):
            buf.append(md_lines[i].strip()); i += 1
        if not buf:
            buf.append(st); i += 1
        txt = " ".join(buf)
        out.append(f"<p{para_class(txt)}>{inline(txt)}</p>")
    return "\n".join(out), toc, subtitle, confirm_items, uc_cols

placed, missing, pending_mascot, referenced = set(), [], [False], set()
def diagram_index(md_lines, svgs):
    order = []
    for l in md_lines:
        m = re.match(r"^\[\[DIAGRAM:\s*([a-z0-9-]+)\s*\]\]$", l.strip())
        if m and m.group(1) in svgs and m.group(1) not in order:
            order.append(m.group(1))
    order += [s for s in svgs if s not in order]
    items = []
    for k, slug in enumerate(order, 1):
        title = svgs[slug][1]
        mnum = re.match(r"Diagram (\d+)\.\s*(.*)", title)
        num, rest = (mnum.group(1), mnum.group(2)) if mnum else ("?", title)
        short = re.split(r"[:(]", rest)[0].strip()
        items.append(f'<li><a href="#fig-{slug}">Diagram {num}</a>: {html.escape(short)} <span class="slug"><code>{slug}</code></span></li>')
    return ('<nav class="callout diagram-index" aria-label="Diagram index"><p><strong>Diagram index (reading order).</strong> '
            'The diagram numbers are stable IDs that the text uses. This list gives them in the order in which they appear in this document.</p>'
            '<ol>' + "".join(items) + '</ol></nav>')

def convert_inner(lines, svgs):
    return convert(lines, svgs)

def figure(slug, svgs, desc):
    svg, title = svgs[slug]
    cap = f'<strong>{html.escape(title)}</strong>'
    if desc:
        cap += f'<span class="figdesc">{inline(desc)}</span>'
    cap += f' <span class="slug">Slug: <code>{slug}</code></span>'
    return f'<figure class="diagram" id="fig-{slug}" data-slug="{slug}">\n<div class="svgwrap">{svg}</div>\n<figcaption>{cap}</figcaption>\n</figure>'

# ---------------- mascot ----------------
STATES = [
    ("idle", "Cyan eyes, calm face. Ready, mic closed; also ask uncertain, needs input and session recording (with overlays)."),
    ("listening", "Green eyes, green listening ring, one arm up. PTT down or held, hands-free capture."),
    ("processing", "Violet eyes and a dot pattern. Transcribing, cleaning, routing, preview, main agent turn, progress, rate-limited."),
    ("answering", "Amber eyes, arm out. Partial answer and blurb reveal (1.2 s or less)."),
    ("success", "Green happy eyes and sparkles. Inserted (1.2 s), completed (2.5 s), memory answer (2.5 s)."),
    ("error", "Red X eyes, dim star. Failed (3 s), login required (3 s); attention if the user must act."),
    ("sleeping", "Closed eyes and Z marks. MOG paused or dozing."),
    ("attention + recording", "Overlays over any mood: a \"?\" bubble (attention, UX-03) and a red ring with a REC dot (recording, UX-04a)."),
]
def mascot_html():
    b64 = base64.b64encode(open(SHEET, "rb").read()).decode()
    css = f".mascot-tile .sprite{{background-image:url(data:image/png;base64,{b64})}}"
    cells = []
    for k, (name, d) in enumerate(STATES):
        col, row = k % 4, k // 4
        cells.append(f'<figure class="mascot-tile"><div class="sprite" role="img" aria-label="MOG mascot, mood {name}: {html.escape(d)}" '
                     f'style="background-position:{col*100/3:.4f}% {row*100}%"></div>'
                     f'<figcaption><strong>{k+1}. <code>{name}</code></strong><br>{html.escape(d)}</figcaption></figure>')
    return (f"<style>{css}</style>"
            '<section class="mascot" aria-labelledby="mascot-h"><h4 id="mascot-h">MOG mascot: the 7 Rive moods and 2 overlays (sprite sheet)</h4>'
            '<p class="note">Source: <code>mog-rive/shots/sheet.png</code> (1600 x 800 px, 4 x 2 grid, the 8th cell shows the two overlays), embedded in this file. The MOG rises out of a Genie lamp (U14); regenerate with <code>tools/render_mog_shots.py</code>. '
            'The mood enum order is idle, listening, processing, answering, success, error, sleeping (Diagram 7).</p>'
            '<div class="mascot-grid">' + "".join(cells) + "</div></section>")
MASCOT_HTML = ""

CSS = r"""
:root{--accent:#FF3621;--accent-ink:#C2260F;--bg:#FFFFFF;--bg2:#F6F7F8;--bg3:#ECEEF0;--fg:#1B1F24;--fg2:#4A525C;--line:#D6DADF;
--must:#B42318;--must-bg:#FEE4E2;--should:#8A4B00;--should-bg:#FEF0C7;--could:#344054;--could-bg:#EAECF0;
--crit:#7A0B0B;--crit-bg:#FDD3D0;--high:#9A2A06;--high-bg:#FFE2CC;--med:#7A5A00;--med-bg:#FEF3C2;--low:#2F5D35;--low-bg:#DDF3E0;
--mvpd:#0B4F8A;--mvpd-bg:#DCEBFA;--mvpm:#5B2A86;--mvpm-bg:#EBDDF7;--v1:#3F4A55;--v1-bg:#E7EAEE;--v2:#55606B;--v2-bg:#F1F3F5;
--confirm:#8A1C00;--confirm-bg:#FFF1EC;--code-bg:#F2F4F6;--row-alt:#FAFBFC;--toc-w:300px}
@media (prefers-color-scheme:dark){:root{--accent:#FF5A45;--accent-ink:#FF8A78;--bg:#14171A;--bg2:#1C2024;--bg3:#262B30;--fg:#E7EAED;--fg2:#AEB6BF;--line:#363C43;
--must:#FFB4AB;--must-bg:#4A1410;--should:#FFD68A;--should-bg:#3D2A05;--could:#D0D5DD;--could-bg:#2D333A;
--crit:#FFC2BC;--crit-bg:#5C0F0A;--high:#FFC9A8;--high-bg:#4D1F06;--med:#FBE08A;--med-bg:#3B3006;--low:#B5E5BC;--low-bg:#173A1D;
--mvpd:#A8D1FA;--mvpd-bg:#0F2E4D;--mvpm:#D9BDF5;--mvpm-bg:#33184D;--v1:#CDD3DA;--v1-bg:#2D333A;--v2:#B6BEC7;--v2-bg:#23282D;
--confirm:#FFC4B5;--confirm-bg:#3A1A12;--code-bg:#22272C;--row-alt:#181B1F}}
*{box-sizing:border-box}
html{scroll-behavior:smooth;scroll-padding-top:12px}
body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.55 system-ui,-apple-system,"Segoe UI",Roboto,"Helvetica Neue",Arial,sans-serif}
a{color:var(--accent-ink)} a:focus-visible,summary:focus-visible{outline:3px solid var(--accent);outline-offset:2px}
.skip{position:absolute;left:-9999px;top:0;background:var(--accent);color:#fff;padding:8px 12px;z-index:10}.skip:focus{left:8px}
.layout{display:grid;grid-template-columns:var(--toc-w) minmax(0,1fr);min-height:100vh}
nav.toc{position:sticky;top:0;height:100vh;overflow:auto;background:var(--bg2);border-right:1px solid var(--line);padding:16px 14px 40px;font-size:13px}
nav.toc .brand{display:flex;gap:8px;align-items:center;font-weight:700;margin-bottom:10px}
nav.toc .brand i{display:inline-block;width:12px;height:12px;border-radius:3px;background:var(--accent)}
nav.toc ol{list-style:none;margin:0;padding:0} nav.toc li{margin:1px 0}
nav.toc a{display:block;color:var(--fg);text-decoration:none;padding:3px 6px;border-radius:5px} nav.toc a:hover{background:var(--bg3)}
nav.toc details>summary{cursor:pointer;list-style:none;display:flex;align-items:center;gap:4px} nav.toc details>summary::-webkit-details-marker{display:none}
nav.toc details>summary::before{content:"▸";color:var(--fg2);width:10px;flex:none;transition:transform .15s}
nav.toc details[open]>summary::before{transform:rotate(90deg)}
nav.toc details>summary a{flex:1;font-weight:600}
nav.toc ol ol{margin:2px 0 6px 14px;border-left:2px solid var(--line);padding-left:6px} nav.toc ol ol a{color:var(--fg2);padding:2px 6px}
nav.toc .quick a{font-weight:700;color:var(--accent-ink)}
main{padding:28px clamp(16px,4vw,56px) 80px;min-width:0}
header.doc h1{font-size:clamp(24px,3vw,34px);line-height:1.2;margin:0 0 6px;border-left:6px solid var(--accent);padding-left:14px}
header.doc .sub{color:var(--fg2);margin:0 0 18px 20px}
h2{font-size:24px;margin:48px 0 12px;padding-bottom:6px;border-bottom:2px solid var(--accent)}
h3{font-size:18px;margin:30px 0 10px} h4{font-size:16px;margin:22px 0 8px}
h2 .anchor,h3 .anchor{color:var(--fg2);text-decoration:none;opacity:0;font-weight:400;margin-left:4px} h2:hover .anchor,h3:hover .anchor{opacity:1}
p,li{max-width:110ch}
code{font:0.88em/1.4 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;background:var(--code-bg);padding:1px 4px;border-radius:4px;overflow-wrap:anywhere}
pre.code{background:var(--code-bg);border:1px solid var(--line);border-radius:8px;padding:12px 14px;overflow:auto;font-size:12.5px;line-height:1.45;position:relative}
pre.code code{background:none;padding:0;overflow-wrap:normal}
pre.code[data-lang]::before{content:attr(data-lang);position:absolute;top:4px;right:8px;font-size:11px;color:var(--fg2)}
details{margin:12px 0} main details>summary{cursor:pointer;font-weight:600;color:var(--accent-ink)}
hr{border:0;border-top:1px solid var(--line);margin:36px 0}
.tw{margin:12px 0 20px}
table{border-collapse:separate;border-spacing:0;width:100%;font-size:13.5px;border:1px solid var(--line);border-radius:8px}
th,td{padding:6px 8px;vertical-align:top;text-align:left;border-bottom:1px solid var(--line);overflow-wrap:anywhere}
thead th{position:sticky;top:0;z-index:2;background:var(--bg3);font-weight:700;border-bottom:2px solid var(--accent);white-space:nowrap}
tbody th{font-weight:600;white-space:nowrap}
tbody tr:nth-child(even){background:var(--row-alt)} tbody tr:hover{background:var(--bg2)}
tbody tr:target{outline:3px solid var(--accent);outline-offset:-3px}
tr.confirm-row{background:var(--confirm-bg)!important;box-shadow:inset 4px 0 0 var(--accent)}
tr.exist-row{background:var(--confirm-bg)!important;box-shadow:inset 4px 0 0 #7A0B0B}
.badge{display:inline-block;font:700 11px/1.6 system-ui,sans-serif;letter-spacing:.03em;padding:0 7px;border-radius:999px;white-space:nowrap;vertical-align:1px}
.badge.pri.must{background:var(--must-bg);color:var(--must);border:1px solid currentColor}
.badge.pri.should{background:var(--should-bg);color:var(--should);border:1px solid currentColor}
.badge.pri.could{background:var(--could-bg);color:var(--could);border:1px dashed currentColor}
.badge.confirm{background:var(--accent);color:#fff;padding:1px 8px;box-shadow:0 0 0 2px var(--confirm-bg)}
@media (prefers-color-scheme:dark){.badge.confirm{background:#C2260F}}
.badge.unv{background:var(--should-bg);color:var(--should);border:1px dashed currentColor}
.chip{display:inline-block;font:600 11px/1.6 system-ui,sans-serif;padding:0 6px;border-radius:5px;white-space:nowrap;margin:1px 0}
.ph-mvpd{background:var(--mvpd-bg);color:var(--mvpd)} .ph-mvpm{background:var(--mvpm-bg);color:var(--mvpm)}
.ph-v1{background:var(--v1-bg);color:var(--v1);border:1px dashed currentColor} .ph-v2{background:var(--v2-bg);color:var(--v2);border:1px dotted currentColor}
.chip.uc{background:var(--bg3);color:var(--fg)}
.sev{display:inline-block;font:700 11px/1.6 system-ui,sans-serif;padding:0 7px;border-radius:4px}
.sev-crit{background:var(--crit-bg);color:var(--crit);border:1px solid currentColor} .sev-high{background:var(--high-bg);color:var(--high)}
.sev-med{background:var(--med-bg);color:var(--med)} .sev-low{background:var(--low-bg);color:var(--low)}
.st{font:600 11px/1.6 system-ui,sans-serif;padding:0 6px;border-radius:4px;background:var(--bg3);white-space:nowrap}
.st-uc{background:var(--accent);color:#fff} .st-spike{background:var(--should-bg);color:var(--should)}
.callout{border:1px solid var(--line);border-left:5px solid var(--fg2);background:var(--bg2);border-radius:8px;padding:10px 14px;margin:14px 0}
blockquote.callout{margin-left:0;margin-right:0}
.callout.existential{border-left-color:#7A0B0B;background:var(--confirm-bg);font-size:16px}
.callout.confirm-box,.has-confirm{border-left:5px solid var(--accent);background:var(--confirm-bg);padding:8px 12px;border-radius:6px}
.callout.warn{border-left-color:var(--accent)}
.panel{border:2px solid var(--accent);border-radius:12px;padding:14px 18px;margin:18px 0;background:var(--confirm-bg)}
.panel h2{margin:0 0 8px;border:0;font-size:20px} .panel ul{columns:2 320px;padding-left:18px;margin:6px 0} .panel li{break-inside:avoid;margin:2px 0}
.ex-box{border:2px solid #7A0B0B;border-radius:10px;padding:10px 14px;margin:10px 0;background:var(--bg)}
.ex-box strong.big{font-size:17px}
.doc-control{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:0;border:1px solid var(--line);border-radius:10px;overflow:hidden;margin:14px 0}
.doc-control div{padding:10px 14px;border-right:1px solid var(--line);background:var(--bg2)} .doc-control dt{font-size:11px;text-transform:uppercase;letter-spacing:.06em;color:var(--fg2)} .doc-control dd{margin:2px 0 0;font-weight:700}
.howto{border:1px dashed var(--fg2);border-radius:10px;padding:12px 16px;margin:14px 0}
.howto h2{margin:0 0 6px;border:0;font-size:18px}
.legend{display:flex;flex-wrap:wrap;gap:6px 14px;align-items:center;margin:6px 0}
nav.diagram-index ol{margin:6px 0 0 1.4em;padding:0}nav.diagram-index li{margin:2px 0}
figure.diagram{margin:22px 0;padding:0;border:1px solid var(--line);border-radius:10px;overflow:hidden;break-inside:avoid}
.svgwrap{background:#FFFFFF;padding:8px} .svgwrap svg{display:block;width:100%;height:auto;max-width:100%}
figure.diagram figcaption{padding:10px 14px;background:var(--bg2);font-size:13.5px;border-top:1px solid var(--line)}
.figdesc{display:block;margin-top:4px;color:var(--fg)} .slug{display:block;color:var(--fg2);font-size:12px;margin-top:4px}
.mascot{margin:20px 0;border:1px solid var(--line);border-radius:10px;padding:12px 14px;background:var(--bg2)} .note{color:var(--fg2);font-size:13px}
.mascot-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(170px,1fr));gap:12px}
.mascot-tile{margin:0;background:var(--bg);border:1px solid var(--line);border-radius:10px;overflow:hidden}
.mascot-tile .sprite{aspect-ratio:1/1;width:100%;background-color:#1E1E1E;background-size:400% 200%;background-repeat:no-repeat;-webkit-print-color-adjust:exact;print-color-adjust:exact}
.mascot-tile figcaption{padding:8px 10px;font-size:12.5px}
footer{margin-top:60px;color:var(--fg2);font-size:12.5px;border-top:1px solid var(--line);padding-top:12px}
@media (max-width:980px){.layout{grid-template-columns:1fr}nav.toc{position:static;height:auto;max-height:none;border-right:0;border-bottom:1px solid var(--line)}
 .tw{overflow-x:auto} thead th{position:static}}
@media print{@page{size:A4;margin:14mm 12mm}
 :root{--bg:#fff;--bg2:#fff;--bg3:#eee;--fg:#000;--fg2:#333;--line:#999;--row-alt:#fff;--confirm-bg:#FFF1EC;--code-bg:#f4f4f4}
 body{font-size:10pt;background:#fff;color:#000}.layout{display:block}nav.toc,.skip,.anchor{display:none!important}
 main{padding:0} thead th{position:static} thead{display:table-header-group} tr,figure,pre,.callout,.panel{break-inside:avoid}
 h2{break-before:page;break-after:avoid} h3,h4{break-after:avoid} details{display:block} details>*{display:block}
 a.cite::after{content:" (" attr(href) ")";font-size:8pt;color:#333} a{color:#000}
 .badge,.chip,.sev,.st,tr.confirm-row,.panel{-webkit-print-color-adjust:exact;print-color-adjust:exact}}
@media (prefers-reduced-motion:reduce){html{scroll-behavior:auto}}
"""

def main():
    global MASCOT_HTML
    svgs = load_svgs()
    MASCOT_HTML = mascot_html()
    lines = open(MD, encoding="utf-8").read().splitlines()
    referenced.update(re.findall(r"^\[\[DIAGRAM:\s*([a-z0-9-]+)\s*\]\]$", "\n".join(lines), re.M))
    body, toc, subtitle, confirm_items, uc_cols = convert(lines, svgs)
    # leftover diagrams if no section 4 boundary caught them
    left = [s for s in svgs if s not in placed]
    if left:
        extra = "\n".join(figure(s, svgs, None) for s in left)
        arch = next((h for t, h, _ in toc if t.startswith("4.")), None)
        mpos = body.find(f'<h2 id="{toc[[h for _, h, _ in toc].index(arch)+1][1]}"') if arch else -1
        body = body[:mpos] + extra + body[mpos:] if mpos > 0 else body + extra
        placed.update(left)

    exist_anchor = next((h for t, h, _ in toc if t.startswith("19.")), "")
    dec_anchor = next((h for t, h, _ in toc if t.startswith("12.")), "")
    seen, ci = set(), []
    # B1 is binding; only its PTT-preview note needs confirmation, which is COL-75 / RTR-04a.
    relabel = {"B1": ("COL-75 / RTR-04a", "keep the 1.5 s PTT preview (B1 itself is binding)", "r-COL-75")}
    for key, topic, rid, sec in confirm_items:
        if key in relabel:
            key, topic, rid = relabel[key]
            seen.add("COL-75")
        if key in seen: continue
        seen.add(key)
        tgt = f"#{rid}" if rid else f"#{dec_anchor}"
        ci.append(f'<li><a href="{tgt}"><strong>{html.escape(key)}</strong></a> {html.escape(topic[:90])}</li>')
    uc = "".join(f'<li><a href="#{r}"><strong>{html.escape(k)}</strong></a> {html.escape(t[:80])}{(" <em>(" + html.escape(stt) + ")</em>") if stt != "UC" else ""}</li>' for k, t, r, stt in uc_cols)
    panel = f"""<section class="panel" aria-labelledby="needs-user">
<h2 id="needs-user">Needs the user: existential questions and decisions to confirm</h2>
<div class="ex-box"><strong class="big">Existential questions: none.</strong> No open question decides whether the project can exist (D83). The reasons are in <a href="#{exist_anchor}">Section 19</a>. This builder found no new existential question.</div>
<p><span class="badge confirm">CONFIRM WITH USER</span> The director or the editor made these decisions. The user must confirm them. The rows are highlighted in the tables. Count: {len(ci)}.</p>
<ul>{''.join(ci)}</ul>
<p>Collisions with status <span class="st st-uc">UC: confirm</span>, alone or compound (decision made, user confirmation needed). Count: {len(uc_cols)}.</p>
<ul>{uc}</ul>
</section>"""

    doc_control = """<section aria-labelledby="dc-h"><h2 id="dc-h" style="margin-top:20px">Document control</h2>
<dl class="doc-control">
<div><dt>Version</dt><dd>1.2</dd></div><div><dt>Date</dt><dd>2026-11-14</dd></div>
<div><dt>Standard</dt><dd>ASD-STE100, Issue 9</dd></div>
<div><dt>Status</dt><dd>Decision-complete. Some items need user confirmation.</dd></div>
<div><dt>Product</dt><dd>Bottle (formerly Mother Bricks) and MOGs</dd></div>
<div><dt>Source</dt><dd><code>research/final-requirements.md</code></dd></div>
</dl></section>"""

    howto = """<section class="howto" aria-labelledby="howto-h"><h2 id="howto-h">How to read this document</h2>
<p>Each requirement is one row with one sentence and one rule: "The &lt;subject&gt; must &lt;verb&gt; &lt;object&gt; [condition]." Each requirement sentence uses "must". The badge in the <strong>Pri</strong> column gives the obligation:</p>
<div class="legend"><span class="badge pri must">MUST</span> Pri M, mandatory <span class="badge pri should">SHOULD</span> Pri S, recommended <span class="badge pri could">COULD</span> Pri C, optional</div>
<p>A SHOULD or COULD row is mandatory only when the team builds that feature. The <strong>Phase</strong> chip tells when the row applies. The MVP exit needs both MVP-D and MVP-M.</p>
<div class="legend"><span class="chip ph-mvpd">MVP-D</span> dictation and Q&amp;A milestone <span class="chip ph-mvpm">MVP-M</span> meeting milestone <span class="chip ph-v1">v1</span> after the MVP <span class="chip ph-v2">v2</span> later</div>
<p>The <strong>UC</strong> chip names the use case: UC1 dictation, UC2 Q&amp;A, UC3 send context to Genie and draft UC Pages, MTG meeting recording, ALL platform. The <strong>Verify</strong> column gives the method (T test, I inspection, D demonstration, A analysis) and the acceptance-test ID (AT-xx, Section 15).</p>
<div class="legend"><span class="badge confirm">CONFIRM WITH USER</span> a decision that the user must confirm (highlighted row) <span class="badge unv">[U]</span> a fact that is not verified; a spike SP-xx closes it</div>
<div class="legend">Collision severity: <span class="sev sev-crit">Crit</span> <span class="sev sev-high">High</span> <span class="sev sev-med">Med</span> <span class="sev sev-low">Low</span> · Status: <span class="st st-res">RES</span> resolved <span class="st st-ar">AR</span> accepted risk <span class="st st-spike">SPIKE</span> open with a spike <span class="st st-uc">UC: confirm</span> needs user confirmation</div>
<p>In the diagrams, solid arrows are requests or data, dashed arrows are events or streams, and dotted arrows are credential fetches. The red dashed border is the secret boundary: no token crosses it. Items for v1 use the "optional / v1" style. Each diagram scales to the page width.</p>
</section>"""

    tocl = ['<ol>', '<li class="quick"><a href="#dc-h">Document control</a></li>', '<li class="quick"><a href="#needs-user">Needs the user</a></li>', '<li class="quick"><a href="#howto-h">How to read</a></li>']
    for t, h, subs in toc:
        tt = html.escape(re.sub(r"[*`]", "", t))
        if subs:
            tocl.append(f'<li><details><summary><a href="#{h}">{tt}</a></summary><ol>' +
                        "".join(f'<li><a href="#{sh}">{html.escape(re.sub(r"[*`]", "", st))}</a></li>' for st, sh in subs) + "</ol></details></li>")
        else:
            tocl.append(f'<li><a href="#{h}">{tt}</a></li>')
    tocl.append("</ol>")

    page = f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>{html.escape(TITLE)}</title>
<style>{CSS}</style>
</head>
<body>
<a class="skip" href="#content">Skip to content</a>
<div class="layout">
<nav class="toc" aria-label="Table of contents"><div class="brand"><i aria-hidden="true"></i>Bottle &amp; MOG requirements</div>
{''.join(tocl)}
</nav>
<main id="content">
<header class="doc"><h1>{html.escape(TITLE)}</h1><p class="sub">{inline(subtitle)}</p></header>
{doc_control}
{panel}
{howto}
{body}
<footer>Generated on 2026-11-14 from <code>research/final-requirements.md</code> (v1.2) and {len(svgs)} SVG diagrams in <code>research/diagrams/</code>. This file is self-contained: it needs no network access. The only external links are source citations.</footer>
</main></div>
</body></html>
"""
    open(OUT, "w", encoding="utf-8").write(page)
    print("wrote", OUT, len(page), "bytes; diagrams placed", sorted(placed), "missing", missing,
          "confirm", len(ci), "uc", len(uc_cols))

if __name__ == "__main__":
    main()
