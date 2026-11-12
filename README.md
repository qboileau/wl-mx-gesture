# wl-mx-gesture

Gestures and an action wheel for the thumb gesture button of the Logitech MX Master 3S / 4,
on any Wayland compositor.

- **Gestures**: tap, double tap, hold, swipe up/down/left/right while holding the button.
- **Wheel**: double tap (default) or hold still to open a wheel of actions around the cursor; keep the button pressed,
  move towards a slot and release to run it (release in the centre to cancel).
- **Actions**: workspace switching, moving/closing/floating/fullscreen windows, special workspaces, shell commands,
  or raw Hyprland dispatches. Hyprland's legacy and Lua (0.55+) IPC syntaxes are auto-detected.

The button is read directly over Logitech's HID++ protocol (`/dev/hidraw*`), through a Bolt receiver or
Bluetooth, so no compositor support is needed for input. While the button is held the cursor stays frozen and the
mouse movement drives the gesture.

The wheel is drawn on a wlr-layer-shell overlay (Hyprland, Sway, niri, KDE, river, labwc, Wayfire, COSMIC…; not
GNOME), at the cursor and kept on screen near edges, with labels, icons from your icon theme and a `[style]`.

> Status: early. Gestures, actions and the wheel work; sub-wheels, hot-reload and the configuration GUI are not built
> yet. See [docs/PLAN.md](docs/PLAN.md).

On Hyprland, layers fade in by default; to show the wheel instantly add a layer rule for the `wl-mx-gesture`
namespace that disables animations.

## Install (Nix flake)

NixOS, which also installs the udev rule granting hidraw access:

```nix
{
  inputs.wl-mx-gesture.url = "github:qboileau/wl-mx-gesture";
  # in your NixOS configuration
  imports = [ inputs.wl-mx-gesture.nixosModules.default ];
  programs.wl-mx-gesture.enable = true;
}
```

Home Manager, to manage the configuration and the user service (set `programs.wl-mx-gesture.service = false` on the
NixOS side to avoid two services):

```nix
imports = [ inputs.wl-mx-gesture.homeManagerModules.default ];
services.wl-mx-gesture = {
  enable = true;
  settings.bindings.swipe.left.hyprland = "workspace e-1";
};
```

Other distributions: build with `cargo build --release` and install `packaging/70-wl-mx-gesture.rules` into
`/etc/udev/rules.d/`.

Do not run logiops or Solaar diversions of the gesture button at the same time.

## Configuration

`~/.config/wl-mx-gesture/config.toml`; when missing, the built-in [config/default.toml](config/default.toml) is used.
It documents every option.

## Troubleshooting

```sh
wl-mx-gesture --probe          # list detected devices and their reprogrammable controls (0x00c3 = gesture button)
wl-mx-gesture --test           # print mouse status and recognised gestures, without running actions
wl-mx-gesture --preview        # show the wheel with its style, cycling the highlight (style tuning)
RUST_LOG=debug wl-mx-gesture   # trace HID++ decisions
```

## Development

```sh
nix develop --command cargo test
nix build
```

## License

MIT
