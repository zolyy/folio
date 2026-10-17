# Acrobat Pro reference — the viewer (stage 4)

What Acrobat Pro's reading window offers, from Adobe's public help (*Viewing PDFs and
viewing preferences*, *Navigating PDF pages*, *Searching PDFs*), and how Folio's desktop app
maps it. "4a…4d" = the sub-stage that brings it, **Folio choice** = not documented publicly.
Folio's look and icons are its own (AGENTS.md §1): toolbar icons are simple vector strokes
drawn by Folio (`apps/folio-app/src/icons.rs`), no font or image asset and no Adobe artwork.

## view.tabs, view.open — documents

| Acrobat | Folio |
|---|---|
| Open (Ctrl+O), recent files, drag and drop | 4a ✓ (recent list: 10 files, **Folio choice**) |
| One tab per document, close tab (Ctrl+W), switch tabs (Ctrl+Tab) | 4a ✓; a file already open comes to front |
| Password prompt for protected files | 4a ✓ |
| Damaged files open with repairs reported | 4a ✓ (status bar: "Repaired: N problem(s)", details on hover) |

## view.navigate — moving around

| Acrobat | Folio |
|---|---|
| Next / previous page, first / last page, go to page number or label | 4a ✓ (labels first, as with "Use logical page numbers") |
| Page navigation keys: arrows, Page Up/Down, Home/End | 4a ✓ |
| Zoom in / out (Ctrl + / Ctrl −), zoom percentage box | 4a ✓ presets 8.33–6400%, typed 1–6400% |
| Fit Page, Fit Width, Actual Size (100%), Fit Visible | 4a ✓ (Fit Visible later). 100% = printed size at 96 px/in ("Use system setting") |
| Rotate View clockwise / counterclockwise (view only, not saved) | 4a ✓ |
| Marquee zoom, Dynamic zoom, Pan & Zoom window | later |

## view.layout — page display

| Acrobat | Folio |
|---|---|
| Single Page View, Enable Scrolling (continuous) | 4a ✓ (default: Enable Scrolling, Fit Page — **Folio choice**) |
| Two Page View, Two Page Scrolling, Show Cover Page | 4c |
| Full Screen Mode (Ctrl+L) | 4c |
| Read Mode (hide toolbars) | 4c |

## view.thumbnails, view.bookmarks — navigation panes

| Acrobat | Folio |
|---|---|
| Page Thumbnails panel, click to go, current page highlighted | 4a ✓ |
| Bookmarks panel: tree, expand/collapse, click to go | 4a ✓ (initially open items from `/Count`) |
| Enlarge / reduce thumbnails | 4c |

## view.search, view.select-copy

| Acrobat | Folio |
|---|---|
| Find (Ctrl+F): next / previous, match count, whole words, case-sensitive | 4b |
| Highlight every match on the page | 4b |
| Select text with the cursor, copy (Ctrl+C), select all (Ctrl+A) | 4b |
| Advanced Search across files | later (tools.index) |

## view.layers, view.attachments, view.properties, view.night-mode

| Acrobat | Folio |
|---|---|
| Layers panel: show / hide optional content | 4c |
| Attachments panel: list, save | 4c |
| Document Properties dialog (Ctrl+D) | 4c (same fields as `folio info`) |
| Accessibility → Replace Document Colors (high contrast, custom) | 4c |

## Agents and automated testing

| Folio | |
|---|---|
| UI state reachable without a screen (egui_kittest + AccessKit) | 4a ✓, used by every UI test; every control has an accessible name |
| Opt-in control channel `--control <file>` (loopback, token) | 4d |

## Keyboard shortcuts (stage 4a, Windows/Linux; Cmd on macOS)

| Shortcut | Command |
|---|---|
| Ctrl+O | Open |
| Ctrl+W | Close |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous document |
| Ctrl+= (Ctrl+Plus) / Ctrl+− | Zoom In / Zoom Out |
| Ctrl+0 / Ctrl+1 / Ctrl+2 | Zoom to Page Level / Actual Size / Fit Width |
| Ctrl+Shift+Plus / Ctrl+Shift+Minus | Rotate View clockwise / counterclockwise |
| Ctrl+Shift+N | Go to Page (focuses the page box) |
| Right Arrow, Page Down / Left Arrow, Page Up | Next / previous page |
| Home / End | First / last page |
| Up / Down Arrow | Scroll |
| F4 | Show / hide the navigation pane |

## Rendering in the window (Folio choices)

- Pages render in the background on up to four threads; visible pages first, then the next two.
- Finished pages are kept up to 768 MB; a lower-resolution image stands in while a sharper one renders.
- A page image never exceeds the display's maximum texture size (nor 8192 px); larger views are
  stretched until tiled rendering arrives.
