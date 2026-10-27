# macOS 27 tray click fix

This is the published `tray-icon` 0.24.1 crate with the macOS menu attachment
fix from [upstream PR #365](https://github.com/tauri-apps/tray-icon/pull/365)
applied to `src/platform_impl/macos/mod.rs`.

macOS 27 sends clicks directly to an `NSStatusItem` with a resident menu, so
the app's tray click handler never receives the left click. Keep the menu
detached until a right click requests it, then detach it after presentation.
This preserves the right-click menu and restores the left-click quota popover.

Remove this local patch once Tauri's compatible `tray-icon` dependency includes
the upstream fix, then update `Cargo.lock` and verify both click paths on macOS.
