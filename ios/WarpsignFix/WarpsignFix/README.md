# WarpsignFix

A library that fixes keychain access group issues in environments where the app's Team ID might differ from the originally expected value, such as when using Sideloading services.

## Features

- Automatically rewrites keychain access group references to use the correct Team ID
- Two remapping strategies available:
  - Team ID-based (default): Dynamically extracts the app's current Team ID and remaps keychain groups
  - App Group-based: Uses the app's first entitlement app group to derive remapping

## Usage

### Basic Usage

The library automatically initializes during the app's startup and begins remapping keychain access groups.

### Selecting Remapping Strategy

```objective-c
// Use Team ID strategy (default)
[WarpsignFix setKeychainRemapStrategy:WSFKeychainRemapStrategyTeamID];

// Use App Group strategy
[WarpsignFix setKeychainRemapStrategy:WSFKeychainRemapStrategyAppGroup];

// Get current strategy
WSFKeychainRemapStrategy currentStrategy = [WarpsignFix currentKeychainRemapStrategy];
```

### Strategies Explained

1. **Team ID Strategy**: Extracts the Team ID from a keychain query, then replaces the first component of any keychain access group with this Team ID.

2. **App Group Strategy**: Takes the first app group from entitlements and uses its domain component for remapping keychain access groups.

## Requirements

- iOS 11.0+
- fishhook library
