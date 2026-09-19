package main

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"errors"
	"image"
	"image/color"
	"image/png"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

func TestSavedImagesAreFinalOutputsAndInlineContent(t *testing.T) {
	outputDirectory := t.TempDir()
	original := testPNG(t, 2, 2)
	response := imageResponse{
		Model:   imageModel,
		Created: 123,
		Data: []imageItem{
			{Base64JSON: base64.StdEncoding.EncodeToString(original)},
			{Base64JSON: base64.StdEncoding.EncodeToString(original)},
		},
	}

	images, err := saveImages(response, map[string]interface{}{"output_dir": outputDirectory})
	if err != nil {
		t.Fatalf("save images: %v", err)
	}
	if len(images) != 2 {
		t.Fatalf("expected 2 saved images, got %d", len(images))
	}
	namePattern := regexp.MustCompile(`^ChatGPT_\d{8}_\d{6}_\d{3}_[12]\.png$`)
	for _, saved := range images {
		if !namePattern.MatchString(filepath.Base(saved.Path)) {
			t.Fatalf("unexpected filename: %s", saved.Path)
		}
		written, readErr := os.ReadFile(saved.Path)
		if readErr != nil {
			t.Fatal(readErr)
		}
		if !bytes.Equal(written, original) || !bytes.Equal(saved.Data, original) {
			t.Fatal("saved and returned image bytes must match the account response")
		}
		if saved.MimeType != "image/png" {
			t.Fatalf("unexpected MIME type: %s", saved.MimeType)
		}
	}

	result := successfulToolResult(response, images)
	content := result["content"].([]contentBlock)
	if len(content) != 3 || content[0].Type != "text" {
		t.Fatalf("unexpected content blocks: %#v", content)
	}
	if !strings.Contains(content[0].Text, "最终交付文件") || !strings.Contains(content[0].Text, "不要复制、移动或重命名") {
		t.Fatalf("missing final-output instruction: %s", content[0].Text)
	}
	for index, block := range content[1:] {
		decoded, decodeErr := base64.StdEncoding.DecodeString(block.Data)
		if decodeErr != nil {
			t.Fatal(decodeErr)
		}
		if block.Type != "image" || block.MimeType != "image/png" || !bytes.Equal(decoded, original) {
			t.Fatalf("unexpected inline image %d: %#v", index, block)
		}
	}
	structured := result["structuredContent"].(savedResult)
	if len(structured.OutputPaths) != 2 || structured.OutputPaths[0] != images[0].Path {
		t.Fatalf("unexpected structured output: %#v", structured)
	}
	structuredJSON, err := json.Marshal(structured)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(structuredJSON, []byte(base64.StdEncoding.EncodeToString(original))) {
		t.Fatal("structuredContent must not duplicate image Base64")
	}
}

func TestLargeImageUsesInMemoryPreview(t *testing.T) {
	original := testPNG(t, 2000, 1000)
	preview, mimeType := inlinePreviewWithLimit(original, "image/png", 1)
	if mimeType != "image/png" {
		t.Fatalf("unexpected preview MIME type: %s", mimeType)
	}
	decoded, _, err := image.Decode(bytes.NewReader(preview))
	if err != nil {
		t.Fatalf("decode preview: %v", err)
	}
	if decoded.Bounds().Dx() != 1536 || decoded.Bounds().Dy() != 768 {
		t.Fatalf("unexpected preview dimensions: %v", decoded.Bounds())
	}
}

func TestImageMIMETypeUsesActualImageBytes(t *testing.T) {
	if got := imageMIMEType(testPNG(t, 1, 1), "image/jpeg", "jpeg"); got != "image/png" {
		t.Fatalf("expected PNG signature to win, got %s", got)
	}
	webp := append([]byte("RIFF1234WEBP"), make([]byte, 8)...)
	if got := imageMIMEType(webp, "", "png"); got != "image/webp" {
		t.Fatalf("expected WebP signature, got %s", got)
	}
	if got := imageMIMEType(nil, "image/jpeg; charset=binary", "png"); got != "image/jpeg" {
		t.Fatalf("expected normalized response MIME type, got %s", got)
	}
}

func TestInstructionsReserveOutputDirectoryForExplicitUserRequests(t *testing.T) {
	if !strings.Contains(serverInstructions, "outputPaths 是最终交付文件") || !strings.Contains(serverInstructions, "不得自动重试") {
		t.Fatalf("incomplete server instructions: %s", serverInstructions)
	}
	encoded, err := json.Marshal(toolList())
	if err != nil {
		t.Fatal(err)
	}
	description := string(encoded)
	for _, expected := range []string{"仅当用户明确指定输出目录时传入", "不要复制、移动或重命名", "直接包含图片预览"} {
		if !strings.Contains(description, expected) {
			t.Fatalf("missing tool instruction %q", expected)
		}
	}
}

func TestInterruptedAccountResponseHasUnknownOutcome(t *testing.T) {
	request, err := http.NewRequest(http.MethodPost, "https://account.example/images/generations", strings.NewReader(`{}`))
	if err != nil {
		t.Fatal(err)
	}
	client := &http.Client{Transport: roundTripFunc(func(*http.Request) (*http.Response, error) {
		return nil, io.ErrUnexpectedEOF
	})}
	_, err = doImageRequestWithClient(client, request)
	var unknownErr unknownOutcomeError
	if !errors.As(err, &unknownErr) {
		t.Fatalf("expected unknownOutcomeError, got %v", err)
	}
	if !strings.Contains(err.Error(), "请勿自动重试") || !strings.Contains(err.Error(), "先征得用户确认") {
		t.Fatalf("unexpected error guidance: %v", err)
	}
}

type roundTripFunc func(*http.Request) (*http.Response, error)

func (function roundTripFunc) RoundTrip(request *http.Request) (*http.Response, error) {
	return function(request)
}

func testPNG(t *testing.T, width, height int) []byte {
	t.Helper()
	canvas := image.NewNRGBA(image.Rect(0, 0, width, height))
	for y := 0; y < height; y++ {
		for x := 0; x < width; x++ {
			canvas.SetNRGBA(x, y, color.NRGBA{R: uint8(x), G: uint8(y), B: uint8(x + y), A: 255})
		}
	}
	var output bytes.Buffer
	if err := png.Encode(&output, canvas); err != nil {
		t.Fatal(err)
	}
	return output.Bytes()
}

func TestLoadAccountConfigUsesClientCodexHome(t *testing.T) {
	dataDirectory := t.TempDir()
	codexHome := filepath.Join(t.TempDir(), "selected-codex-home")
	if err := os.MkdirAll(codexHome, 0o700); err != nil {
		t.Fatal(err)
	}
	writeJSON(t, filepath.Join(dataDirectory, "settings.json"), map[string]interface{}{
		"schema_version": 1,
		"codex_home":     codexHome,
	})
	if err := os.WriteFile(filepath.Join(codexHome, "config.toml"), []byte(`
model_provider = "account-provider"

[model_providers.account-provider]
base_url = "https://account.example/v1/"
experimental_bearer_token = "matching-key"
`), 0o600); err != nil {
		t.Fatal(err)
	}
	writeJSON(t, filepath.Join(codexHome, "auth.json"), map[string]interface{}{
		"OPENAI_API_KEY": "matching-key",
	})
	t.Setenv("CODEX_HOME", filepath.Join(t.TempDir(), "must-not-be-used"))

	config, err := loadAccountConfigFromDataDir(dataDirectory)
	if err != nil {
		t.Fatalf("load account config: %v", err)
	}
	if config.baseURL != "https://account.example/v1" || config.apiKey != "matching-key" {
		t.Fatalf("unexpected config: %#v", config)
	}
}

func TestLoadAccountConfigRejectsMissingOrMismatchedClientSettings(t *testing.T) {
	t.Run("missing codex_home", func(t *testing.T) {
		dataDirectory := t.TempDir()
		writeJSON(t, filepath.Join(dataDirectory, "settings.json"), map[string]interface{}{
			"schema_version": 1,
		})
		assertAccountUnavailable(t, dataDirectory)
	})

	t.Run("mismatched key", func(t *testing.T) {
		dataDirectory := t.TempDir()
		codexHome := t.TempDir()
		writeJSON(t, filepath.Join(dataDirectory, "settings.json"), map[string]interface{}{
			"codex_home": codexHome,
		})
		if err := os.WriteFile(filepath.Join(codexHome, "config.toml"), []byte(`
model_provider = "account-provider"

[model_providers.account-provider]
base_url = "https://account.example/v1"
experimental_bearer_token = "config-key"
`), 0o600); err != nil {
			t.Fatal(err)
		}
		writeJSON(t, filepath.Join(codexHome, "auth.json"), map[string]interface{}{
			"OPENAI_API_KEY": "different-key",
		})
		assertAccountUnavailable(t, dataDirectory)
	})
}

func assertAccountUnavailable(t *testing.T, dataDirectory string) {
	t.Helper()
	_, err := loadAccountConfigFromDataDir(dataDirectory)
	var unavailable accountUnavailableError
	if !errors.As(err, &unavailable) {
		t.Fatalf("expected accountUnavailableError, got %v", err)
	}
}

func writeJSON(t *testing.T, path string, value interface{}) {
	t.Helper()
	bytes, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, bytes, 0o600); err != nil {
		t.Fatal(err)
	}
}
