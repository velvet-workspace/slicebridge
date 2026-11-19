package main

import (
	"context"
	"embed"
	"fmt"
	"os"

	"github.com/nocturney/slicebridge/engine/bridge"
	"github.com/wailsapp/wails/v2"
	"github.com/wailsapp/wails/v2/pkg/options"
	"github.com/wailsapp/wails/v2/pkg/options/assetserver"
	"github.com/wailsapp/wails/v2/pkg/options/mac"
)

//go:embed all:frontend/dist
var assets embed.FS

func main() {
	// The actual packaged executable probes its adjacent Rust-native worker
	// without launching a GUI, slicing a project, or writing G-code.
	if len(os.Args) == 2 && os.Args[1] == "--verify-bundled-rust" {
		if err := bridge.VerifyRustNativeBundle(); err != nil {
			fmt.Fprintln(os.Stderr, "RUST_NATIVE_BUNDLE_FAILED", err)
			os.Exit(2)
		}
		fmt.Println("RUST_NATIVE_BUNDLE_OK")
		return
	}
	// Headless release smoke probe. The Windows single-file executable must
	// initialize its embedded Prusa engine without opening a WebView.
	if len(os.Args) == 2 && os.Args[1] == "--verify-bundled-prusa" {
		exe, source, reason := locatePrusaEngine()
		if source != "embedded" || exe == "" || reason != "" {
			fmt.Fprintln(os.Stderr, "PRUSA_BUNDLE_FAILED", reason)
			os.Exit(2)
		}
		fmt.Println("PRUSA_BUNDLE_OK")
		return
	}
	app := &App{}
	preferences, prefErr := loadSettings()
	if prefErr != nil {
		fmt.Println("SliceBridge: could not restore settings:", prefErr)
		preferences = defaultSettings()
	}
	err := wails.Run(&options.App{
		Title:            "SliceBridge",
		Width:            preferences.Window.Width,
		Height:           preferences.Window.Height,
		MinWidth:         720,
		MinHeight:        500,
		BackgroundColour: &options.RGBA{R: 247, G: 246, B: 242, A: 255},
		AssetServer:      &assetserver.Options{Assets: assets},
		OnStartup: func(ctx context.Context) {
			app.startup(ctx)
			externalBridge.setContext(ctx)
			for _, raw := range os.Args[1:] {
				externalBridge.acceptArg(raw)
			}
		},
		Mac: &mac.Options{OnUrlOpen: externalBridge.acceptArg},
		SingleInstanceLock: &options.SingleInstanceLock{
			UniqueId: "velvetfactory-slicebridge",
			OnSecondInstanceLaunch: func(info options.SecondInstanceData) {
				for _, raw := range info.Args {
					externalBridge.acceptArg(raw)
				}
			},
		},
		OnBeforeClose: func(_ context.Context) bool {
			// Flush the final user-chosen size even if the 900ms JS debounce
			// has not fired yet; never prevent the OS close command.
			_ = app.RememberWindowSize()
			return false
		},
		Bind:        []interface{}{app},
		DragAndDrop: &options.DragAndDrop{EnableFileDrop: true, DisableWebViewDrop: true},
	})
	if err != nil {
		fmt.Println("SliceBridge:", err)
	}
}
