# Fixtures

Upstream responses used by the widget unit tests.

Everything here was recorded from a live instance on 2026-11-11 and then
scrubbed: names and titles replaced with placeholders, long lists trimmed to a
few entries, unused fields dropped. Counts were left as recorded where the
list was not trimmed.

One exception: `rutorrent/list.json` is hand-written from the ruTorrent
`httprpc` plugin's `mode=list` format (34 positional fields per torrent),
because the recorded instance was answering HTTP 500 at the time.
