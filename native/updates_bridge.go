package main

import (
	"context"
	"fmt"
	"runtime"

	"github.com/nocturney/slicebridge/native/updates"
	wailsruntime "github.com/wailsapp/wails/v2/pkg/runtime"
)

// appVersion identifies the installed development build or published Release.
// Development snapshots increment .dev.N and may be published as prereleases.
// CI may stamp buildCommit (the originating GitHub commit) via -ldflags.
var appVersion = "0.4.0-beta.6.dev.3"
var buildCommit = ""

type AppVersion struct {
	Version  string `json:"version"`
	Commit   string `json:"commit,omitempty"`
	Platform string `json:"platform"`
}

func (a *App) GetVersion() AppVersion {
	return AppVersion{
		Version: appVersion, Commit: buildCommit,
		Platform: runtime.GOOS + "-" + runtime.GOARCH,
	}
}

// CheckForUpdates checks published GitHub Releases only on demand.
// It never downloads, installs, schedules or executes any update.
func (a *App) CheckForUpdates() (updates.Result, error) {
	ctx := a.ctx
	if ctx == nil {
		ctx = context.Background()
	}
	return updates.CheckInstalled(ctx, appVersion)
}

// OpenUpdatePage opens only our own trusted GitHub Releases / Actions pages.
// Never accept an untrusted URL from either the renderer or API JSON.
func (a *App) OpenUpdatePage(kind, tag string) error {
	var dest string
	switch kind {
	case "release":
		page, err := updates.ReleaseURL(tag)
		if err != nil {
			return err
		}
		dest = page
	case "releases":
		dest = updates.ReleasesURL
	case "builds":
		dest = updates.BuildsURL
	default:
		return fmt.Errorf("invalid update page")
	}
	if a.ctx == nil {
		return fmt.Errorf("native desktop window is not ready")
	}
	wailsruntime.BrowserOpenURL(a.ctx, dest)
	return nil
}
