// Command ipatool-bridge is ipatool's App Store client
// (https://github.com/majd/ipatool, MIT) as a C library for the iOS app,
// which can't run the ipatool program the way the CLI does.
//
// One call, JSON in and JSON out. Replies have the shapes ipatool's own
// `--format json` prints, so warpsign-core reads both alike
// (warpsign_core::appstore::ipatool). Failures are
// {"level":"error","error":"…","success":false}.
//
// ipatool is a Go module dependency (go.mod), not a copy: updating it is
// `go get github.com/majd/ipatool/v2@latest`.
package main

/*
#include <stdlib.h>
*/
import "C"

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"unsafe"

	"github.com/byteness/keyring"
	cookiejar "github.com/juju/persistent-cookiejar"
	"github.com/majd/ipatool/v2/pkg/appstore"
	"github.com/majd/ipatool/v2/pkg/keychain"
	"github.com/majd/ipatool/v2/pkg/util/operatingsystem"
)

type request struct {
	Op string `json:"op"`

	// configure
	Home       string `json:"home"`
	Passphrase string `json:"passphrase"`
	MacAddress string `json:"mac_address"`

	// login
	Email    string `json:"email"`
	Password string `json:"password"`
	AuthCode string `json:"auth_code"`

	// search
	Term  string `json:"term"`
	Limit int64  `json:"limit"`

	// versions, download
	BundleID string `json:"bundle_id"`
	AppID    int64  `json:"app_id"`
	Version  string `json:"version"`
	Output   string `json:"output"`
	Purchase bool   `json:"purchase"`

	Platform string `json:"platform"`
}

type reply map[string]any

var (
	mu    sync.Mutex
	store appstore.AppStore
)

//export warpsign_ipatool_call
func warpsign_ipatool_call(input *C.char) *C.char {
	return C.CString(handle(C.GoString(input)))
}

//export warpsign_ipatool_free
func warpsign_ipatool_free(output *C.char) {
	C.free(unsafe.Pointer(output))
}

func main() {}

func handle(input string) (output string) {
	defer func() {
		if r := recover(); r != nil {
			output = failure(fmt.Errorf("ipatool panicked: %v", r))
		}
	}()

	var req request
	if err := json.Unmarshal([]byte(input), &req); err != nil {
		return failure(fmt.Errorf("bad request: %w", err))
	}

	mu.Lock()
	defer mu.Unlock()

	result, err := dispatch(req)
	if err != nil {
		return failure(err)
	}

	result["level"] = "info"
	result["success"] = true
	data, err := json.Marshal(result)
	if err != nil {
		return failure(err)
	}

	return string(data)
}

func failure(err error) string {
	data, _ := json.Marshal(reply{"level": "error", "error": err.Error(), "success": false})
	return string(data)
}

func dispatch(req request) (reply, error) {
	if req.Op == "configure" {
		return configure(req)
	}

	if store == nil {
		return nil, errors.New("ipatool bridge isn't configured")
	}

	platform, err := appstore.ParsePlatform(req.Platform)
	if err != nil {
		return nil, err
	}

	switch req.Op {
	case "prepare":
		// Loads ipatool's runtime and caches Apple's assets; no account needed
		if err := appstore.Prepare(context.Background()); err != nil {
			return nil, err
		}

		return reply{}, nil
	case "login":
		out, err := store.Login(appstore.LoginInput{Email: req.Email, Password: req.Password, AuthCode: req.AuthCode})
		if err != nil {
			return nil, err
		}

		return reply{"name": out.Account.Name, "email": out.Account.Email}, nil
	case "account":
		info, err := store.AccountInfo()
		if err != nil {
			return nil, err
		}

		return reply{"name": info.Account.Name, "email": info.Account.Email}, nil
	case "logout":
		return reply{}, store.Revoke()
	case "search":
		return search(req, platform)
	case "versions":
		return versions(req, platform)
	case "download":
		return download(req, platform)
	default:
		return nil, fmt.Errorf("unknown op %q", req.Op)
	}
}

func configure(req request) (reply, error) {
	if req.Home == "" || req.Passphrase == "" || req.MacAddress == "" {
		return nil, errors.New("configure needs home, passphrase and mac_address")
	}

	dir := filepath.Join(req.Home, "ipatool")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, err
	}

	jar, err := cookiejar.New(&cookiejar.Options{Filename: filepath.Join(dir, "cookies")})
	if err != nil {
		return nil, err
	}

	// An encrypted file, its passphrase kept in the iOS Keychain by the app
	ring, err := keyring.Open(keyring.Config{
		AllowedBackends:  []keyring.BackendType{keyring.FileBackend},
		ServiceName:      "ipatool-auth.service",
		FileDir:          dir,
		FilePasswordFunc: func(string) (string, error) { return req.Passphrase, nil },
	})
	if err != nil {
		return nil, err
	}

	store = appstore.NewAppStore(appstore.Args{
		Keychain:        keychain.New(keychain.Args{Keyring: ring, Label: "ipatool-auth.service"}),
		CookieJar:       jar,
		OperatingSystem: operatingsystem.New(),
		Machine:         machine{home: req.Home, mac: req.MacAddress},
	})

	return reply{}, nil
}

// iOS gives apps no MAC address (they all read 02:00:00:00:00:00), so the
// app makes one up once and keeps it: Apple sees the same device each time.
type machine struct {
	home string
	mac  string
}

func (m machine) MacAddress() (string, error) { return m.mac, nil }
func (m machine) HomeDirectory() string       { return m.home }
func (m machine) ReadPassword(int) ([]byte, error) {
	return nil, errors.New("no terminal")
}

func search(req request, platform appstore.Platform) (reply, error) {
	info, err := store.AccountInfo()
	if err != nil {
		return nil, err
	}

	limit := req.Limit
	if limit <= 0 {
		limit = 10
	}

	out, err := store.Search(appstore.SearchInput{Account: info.Account, Term: req.Term, Limit: limit, Platform: platform})
	if err != nil {
		return nil, err
	}

	apps := make([]reply, 0, len(out.Results))
	for _, app := range out.Results {
		apps = append(apps, reply{
			"id": app.ID, "bundleID": app.BundleID, "name": app.Name,
			"version": app.Version, "price": app.Price,
		})
	}

	return reply{"count": out.Count, "apps": apps}, nil
}

// As ipatool's list-versions command
func versions(req request, platform appstore.Platform) (reply, error) {
	return withAccount(func(acc appstore.Account) (reply, error) {
		app, err := lookup(acc, req, platform)
		if err != nil {
			return nil, err
		}

		out, err := store.ListVersions(appstore.ListVersionsInput{
			Context: context.Background(), Account: acc, App: app, Platform: platform,
		})
		if err != nil {
			return nil, err
		}

		return reply{"externalVersionIdentifiers": out.ExternalVersionIdentifiers, "bundleID": app.BundleID}, nil
	})
}

// As ipatool's download command
func download(req request, platform appstore.Platform) (reply, error) {
	purchased := false

	return withAccount(func(acc appstore.Account) (reply, error) {
		app, err := lookup(acc, req, platform)
		if err != nil {
			return nil, err
		}

		input := appstore.DownloadInput{
			Context: context.Background(), Account: acc, App: app,
			OutputPath: req.Output, ExternalVersionID: req.Version, Platform: platform,
		}

		out, err := store.Download(input)
		if errors.Is(err, appstore.ErrLicenseRequired) && req.Purchase && !purchased {
			err = store.Purchase(appstore.PurchaseInput{Account: acc, App: app, Platform: platform})
			if err != nil && !errors.Is(err, appstore.ErrLicenseAlreadyExists) {
				return nil, err
			}

			purchased = true
			out, err = store.Download(input)
		}

		if err != nil {
			return nil, err
		}

		if platform != appstore.PlatformMacOS || len(out.Sinfs) > 0 {
			err = store.ReplicateSinf(appstore.ReplicateSinfInput{Sinfs: out.Sinfs, PackagePath: out.DestinationPath})
			if err != nil {
				return nil, err
			}
		}

		return reply{"output": out.DestinationPath, "purchased": purchased}, nil
	})
}

// Run with the signed-in account, signing in again once if the store says
// its token has expired (ipatool keeps the password for this)
func withAccount(run func(appstore.Account) (reply, error)) (reply, error) {
	info, err := store.AccountInfo()
	if err != nil {
		return nil, err
	}

	out, err := run(info.Account)
	if !errors.Is(err, appstore.ErrPasswordTokenExpired) {
		return out, err
	}

	login, err := store.Login(appstore.LoginInput{Email: info.Account.Email, Password: info.Account.Password})
	if err != nil {
		return nil, err
	}

	return run(login.Account)
}

func lookup(acc appstore.Account, req request, platform appstore.Platform) (appstore.App, error) {
	app := appstore.App{ID: req.AppID, BundleID: req.BundleID}
	if req.BundleID == "" {
		return app, nil
	}

	out, err := store.Lookup(appstore.LookupInput{Account: acc, BundleID: req.BundleID, Platform: platform})
	// Delisted apps may still download by their numeric ID
	if errors.Is(err, appstore.ErrAppNotFound) && req.AppID != 0 {
		return app, nil
	}

	if err != nil {
		return appstore.App{}, err
	}

	return out.App, nil
}
