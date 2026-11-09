// Added to UnfairPlay by warpsign's tools/unfairplay-credential/build.sh.
// build.sh also registers newExportCredentialCommand() in main.go's
// root.AddCommand(...) call, so no hand-editing is needed.

package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/DrWhax/unfairplay/pkg/fairplay"
	"github.com/spf13/cobra"
)

func newExportCredentialCommand() *cobra.Command {
	var modelFlag, outputFlag string
	var force bool
	command := &cobra.Command{
		Use:   "export-credential",
		Short: "Write the 16-byte account credential from a version-2 model (for fairplay-decrypt / warpsign)",
		RunE: func(_ *cobra.Command, _ []string) error {
			path, err := selectedModelFile(modelFlag)
			if err != nil {
				return err
			}
			credential, err := fairplay.CredentialFromModelFile(path)
			if err != nil {
				return err
			}
			if outputFlag == "" {
				outputFlag = "credential.bin"
			}
			openFlags := os.O_WRONLY | os.O_CREATE | os.O_TRUNC
			if !force {
				openFlags |= os.O_EXCL
			}
			file, err := os.OpenFile(outputFlag, openFlags, 0o600)
			if err != nil {
				return fmt.Errorf("open %s: %w", outputFlag, err)
			}
			defer file.Close()
			if _, err := file.Write(credential[:]); err != nil {
				return fmt.Errorf("write %s: %w", outputFlag, err)
			}
			fmt.Fprintf(os.Stderr, "wrote %s (16-byte account credential; keep it private)\n", outputFlag)
			return nil
		},
	}
	flags := command.Flags()
	flags.StringVar(&modelFlag, "model", "", "Enrollment model .bin file or directory (defaults to the single saved model)")
	flags.StringVar(&outputFlag, "output", "", "Credential output path (default credential.bin)")
	flags.BoolVar(&force, "force", false, "Overwrite an existing output file")
	return command
}

// selectedModelFile resolves a single enrollment model .bin: the --model file if
// given, otherwise the one model in --model's directory or the saved directory
// (defaultModelDirectory lives in setup.go in this package).
func selectedModelFile(explicit string) (string, error) {
	if explicit != "" {
		info, err := os.Stat(explicit)
		if err != nil {
			return "", err
		}
		if !info.IsDir() {
			return explicit, nil
		}
		return singleModelInDirectory(explicit)
	}
	directory, err := defaultModelDirectory()
	if err != nil {
		return "", err
	}
	return singleModelInDirectory(directory)
}

func singleModelInDirectory(directory string) (string, error) {
	entries, err := os.ReadDir(directory)
	if err != nil {
		return "", fmt.Errorf("no saved enrollment models in %s (run `unfairplay setup` or pass --model): %w", directory, err)
	}
	var files []string
	for _, entry := range entries {
		if !entry.IsDir() && strings.EqualFold(filepath.Ext(entry.Name()), ".bin") {
			files = append(files, filepath.Join(directory, entry.Name()))
		}
	}
	switch len(files) {
	case 0:
		return "", fmt.Errorf("no .bin enrollment models in %s", directory)
	case 1:
		return files[0], nil
	default:
		return "", fmt.Errorf("%d enrollment models in %s; pass --model <file> to choose one", len(files), directory)
	}
}
