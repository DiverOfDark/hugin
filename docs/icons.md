# Icons

The `icon` annotation takes one of these references.

| Reference | Source |
| --- | --- |
| `https://...` or `/path` | Used as written. |
| `name.png`, `name.svg`, `name.webp`, or `name` | [dashboard-icons](https://github.com/homarr-labs/dashboard-icons) (`name` means png). |
| `mdi-name` | Material Design Icons. |
| `si-name` | Simple Icons. |

These icons are loaded by the browser, not by Hugin.

A card without an `icon` shows the favicon of the page it links to. Hugin asks that page for its `<link rel="icon">` tags, then for `/favicon.ico`, keeps the picture in memory for a day and serves it from `/icons/<id>`. It only takes pictures from the service's own host, up to 512 kB, and only bytes that are a PNG, ICO, SVG, GIF, JPEG or WebP, so a login page is never mistaken for an icon. A service that has no favicon, or is behind a login Hugin cannot pass, shows its initial. See `favicons` under [Configuration](configuration.md).
