# Connection Dashboard

A native Linux connection dashboard written in Rust. It shows the health of the three links
between your machine and the internet:

```
[ Desktop ] --> [ Router ] --> [ Internet ]
```

For each link it shows the relevant IP address, whether the link is up, and a plain-language
diagnosis when something is wrong. Detected Ethernet and Wi-Fi interfaces are listed with their
current state.

## Why the rewrite

The Qt/Python version blocked its own event loop while it resolved the public IP: on a machine
with no connection, the window could take seconds to appear (or look frozen). This version:

- **opens immediately, online or offline.** No DNS, ICMP, HTTP or subprocess call runs on the UI
  thread. A single background thread refreshes a snapshot every 5 seconds, and the UI only reads
  the latest finished snapshot.
- **stays responsive while probing.** A slow or unreachable network never stalls drawing, input,
  resizing or the theme toggle.
- **skips network-dependent work when offline.** The public-IP request is only attempted after
  the internet probe succeeds, so an offline machine does no DNS lookups or HTTP waits at all.
- **reacts quickly.** The GUI is [egui](https://github.com/emilk/egui) with
  [eframe](https://github.com/emilk/egui/tree/main/crates/eframe), an immediate-mode toolkit that
  only repaints when something changes. Shared controls come from
  [`craft-libs`](https://github.com/storytold/craft-libs) (`craft-ui`): its `Section`, `IconButton`
  and `WidgetPalette` components provide the grouped detail panel, accessible icon buttons and
  consistent widget styling.

## Build and run

Requires Rust 1.95 or newer and the usual Linux desktop build tooling for `winit`/`glutin`.
Internet access is needed for the first build (Cargo fetches crates and the pinned `craft-libs`
revision); the built application itself runs fully offline.

```sh
cargo build --release
./target/release/connection-dashboard
```

`Cargo.lock` is committed. `craft-ui` is pinned to a specific revision so a later change on
`craft-libs`' `main` branch cannot break the build. craft-ui's controls use the Craft suite's
patched `egui`/`winit`, so this project repeats the same immutable `[patch.crates-io]` pins that
`craft-libs` uses in its own workspace (Cargo does not inherit patches from dependencies).

## How the checks work

| Link | Detection |
| --- | --- |
| Desktop | IPv4 address from the routing table (`UDP connect` trick), falling back to `ip addr` |
| Router | Gateway from `ip route`, falling back to `/proc/net/route`; reachable by ICMP, then TCP ports 80/443/53 |
| Internet | TCP 443 to `1.1.1.1`/`8.8.8.8` (ICMP fallback), so networks that drop ping still work |

Ethernet follows the kernel's `LOWER_UP` link flag (`Plugged in` / `Plugged out`). Wi-Fi is
connected when the interface is `UP` and has an IPv4 address.

The public IP is fetched over a short-timeout HTTP request to `api.ipify.org`, with
`checkip.amazonaws.com` and `icanhazip.com` as fallbacks. Only the address is used, and only for
display.

Press the refresh button to probe immediately; otherwise the snapshot refreshes every five
seconds. The sun/moon-style button switches between light and dark themes (the initial theme
follows the desktop preference).
