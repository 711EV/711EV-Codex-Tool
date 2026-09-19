package main

import (
	"bufio"
	"bytes"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"image/png"
	"io"
	"math"
	"mime/multipart"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"time"

	"golang.org/x/image/draw"
	_ "golang.org/x/image/webp"
)

const (
	imageModel              = "gpt-image-2"
	defaultImageSize        = "1024x1024"
	requestTimeout          = 120 * time.Second
	serverVersion           = "2.1.0"
	inlineOriginalByteLimit = 8 * 1024 * 1024
	previewMaxDimension     = 1536
	serverInstructions      = "图片由本服务器保存。返回的 outputPaths 是最终交付文件；除非用户明确指定其他目录，否则不要复制、移动或重命名。工具结果中的 image 内容可直接用于对话预览。遇到结果状态未知的错误时不得自动重试，应先询问用户。"
)

type rpcRequest struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id"`
	Method  string          `json:"method"`
	Params  json.RawMessage `json:"params"`
}

type rpcResponse struct {
	JSONRPC string      `json:"jsonrpc"`
	ID      interface{} `json:"id"`
	Result  interface{} `json:"result,omitempty"`
}

type contentBlock struct {
	Type     string `json:"type"`
	Text     string `json:"text,omitempty"`
	Data     string `json:"data,omitempty"`
	MimeType string `json:"mimeType,omitempty"`
}

type rpcErrorResult struct {
	IsError bool           `json:"isError"`
	Content []contentBlock `json:"content"`
}

type imageResponse struct {
	Created int64           `json:"created,omitempty"`
	Data    []imageItem     `json:"data"`
	Model   string          `json:"model,omitempty"`
	Usage   json.RawMessage `json:"usage,omitempty"`
}

type imageItem struct {
	URL        string `json:"url,omitempty"`
	Base64JSON string `json:"b64_json,omitempty"`
}

type savedResult struct {
	Model       string          `json:"model"`
	Created     int64           `json:"created,omitempty"`
	OutputPaths []string        `json:"outputPaths"`
	Usage       json.RawMessage `json:"usage,omitempty"`
}

type savedImage struct {
	Path     string
	Data     []byte
	MimeType string
}

type accountUnavailableError struct{}

func (accountUnavailableError) Error() string { return "当前账号无法调用" }

type unknownOutcomeError struct {
	cause error
}

func (errorValue unknownOutcomeError) Error() string {
	return "账号可能已经受理本次生成，但 MCP 未收到完整图片结果。请勿自动重试；如需再次生成，请先征得用户确认。原因：" + errorValue.cause.Error()
}

func (errorValue unknownOutcomeError) Unwrap() error { return errorValue.cause }

func main() {
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 64*1024), 16*1024*1024)
	for scanner.Scan() {
		handleMessage(scanner.Bytes())
	}
}

func send(id interface{}, result interface{}) {
	_ = json.NewEncoder(os.Stdout).Encode(rpcResponse{JSONRPC: "2.0", ID: id, Result: result})
}

func sendError(id interface{}, err error) {
	message := "生图失败: " + err.Error()
	var accountErr accountUnavailableError
	var unknownErr unknownOutcomeError
	if errors.As(err, &accountErr) {
		message = accountErr.Error()
	} else if errors.As(err, &unknownErr) {
		message = unknownErr.Error()
	}
	send(id, rpcErrorResult{IsError: true, Content: []contentBlock{{Type: "text", Text: message}}})
}

func handleMessage(raw []byte) {
	var request rpcRequest
	if json.Unmarshal(raw, &request) != nil || request.Method == "" {
		return
	}
	var id interface{}
	if len(request.ID) > 0 && string(request.ID) != "null" {
		_ = json.Unmarshal(request.ID, &id)
	}
	switch request.Method {
	case "initialize":
		send(id, map[string]interface{}{
			"protocolVersion": "2024-11-05",
			"capabilities":    map[string]interface{}{"tools": map[string]interface{}{}},
			"serverInfo":      map[string]string{"name": "generate_image", "version": serverVersion},
			"instructions":    serverInstructions,
		})
	case "notifications/initialized":
	case "ping":
		send(id, map[string]interface{}{})
	case "tools/list":
		send(id, map[string]interface{}{"tools": toolList()})
	case "tools/call":
		var params struct {
			Name      string                 `json:"name"`
			Arguments map[string]interface{} `json:"arguments"`
		}
		if err := json.Unmarshal(request.Params, &params); err != nil {
			sendError(id, errors.New("调用参数格式不正确"))
			return
		}
		result, err := callTool(params.Name, params.Arguments)
		if err != nil {
			sendError(id, err)
			return
		}
		send(id, result)
	}
}

func toolList() []map[string]interface{} {
	common := map[string]interface{}{
		"prompt":             map[string]interface{}{"type": "string", "description": "创建或修改图片的文字描述"},
		"output_dir":         map[string]interface{}{"type": "string", "description": "仅当用户明确指定输出目录时传入；否则必须省略并使用 Windows/macOS 的 Pictures 文件夹"},
		"size":               map[string]interface{}{"type": "string", "description": "图片尺寸，例如 1024x1024、1536x1024、1024x1536 或 auto"},
		"quality":            map[string]interface{}{"type": "string", "description": "图片质量，例如 low、medium、high 或 auto"},
		"background":         map[string]interface{}{"type": "string", "description": "背景参数，例如 auto、opaque 或 transparent"},
		"output_format":      map[string]interface{}{"type": "string", "description": "输出格式：png、jpeg 或 webp"},
		"output_compression": map[string]interface{}{"type": "integer", "minimum": 0, "maximum": 100, "description": "jpeg/webp 压缩质量 0-100"},
		"moderation":         map[string]interface{}{"type": "string", "description": "内容审核级别，例如 auto 或 low"},
		"input_fidelity":     map[string]interface{}{"type": "string", "description": "修改图片时的输入保真度，例如 low 或 high"},
		"response_format":    map[string]interface{}{"type": "string", "description": "账号接口返回格式：url 或 b64_json"},
		"user":               map[string]interface{}{"type": "string", "description": "最终用户标识，可选"},
	}
	generateProperties := cloneMap(common)
	generateProperties["n"] = map[string]interface{}{"type": "integer", "minimum": 1, "maximum": 10, "description": "生成数量，默认 1"}
	editProperties := cloneMap(common)
	editProperties["image_path"] = map[string]interface{}{"type": "string", "description": "待修改的本地图片路径（单张）"}
	editProperties["image_paths"] = map[string]interface{}{"type": "array", "items": map[string]interface{}{"type": "string"}, "description": "待修改的图片路径（多张）"}
	editProperties["mask_path"] = map[string]interface{}{"type": "string", "description": "可选蒙版图片路径"}
	return []map[string]interface{}{
		{"name": "generate_image", "description": "使用 " + imageModel + " 创建图片。图片保存为 ChatGPT_时间戳 文件名，返回的 outputPaths 是最终交付文件；除非用户明确指定 output_dir，否则不要复制、移动或重命名。工具结果会直接包含图片预览。", "inputSchema": map[string]interface{}{"type": "object", "properties": generateProperties, "required": []string{"prompt"}}},
		{"name": "edit_image", "description": "使用 " + imageModel + " 修改一张或多张图片。图片保存为 ChatGPT_时间戳 文件名，返回的 outputPaths 是最终交付文件；除非用户明确指定 output_dir，否则不要复制、移动或重命名。工具结果会直接包含图片预览。", "inputSchema": map[string]interface{}{"type": "object", "properties": editProperties, "required": []string{"prompt"}, "anyOf": []map[string]interface{}{{"required": []string{"image_path"}}, {"required": []string{"image_paths"}}}}},
	}
}

func cloneMap(source map[string]interface{}) map[string]interface{} {
	result := make(map[string]interface{}, len(source))
	for key, value := range source {
		result[key] = value
	}
	return result
}

func callTool(name string, args map[string]interface{}) (map[string]interface{}, error) {
	if name != "generate_image" && name != "edit_image" {
		return nil, fmt.Errorf("未知工具：%s", name)
	}
	prompt := stringArg(args, "prompt")
	if prompt == "" {
		return nil, errors.New("prompt 不能为空")
	}
	images := imagePaths(args)
	isEdit := name == "edit_image" || stringArg(args, "operation") == "edit" || len(images) > 0
	if isEdit && len(images) == 0 {
		return nil, errors.New("修改图片时必须提供 image_path 或 image_paths")
	}
	config, err := loadAccountConfig()
	if err != nil {
		return nil, err
	}
	var response imageResponse
	if isEdit {
		response, err = callEdits(config, args, prompt, images)
	} else {
		response, err = callGenerations(config, args, prompt)
	}
	if err != nil {
		return nil, err
	}
	savedImages, err := saveImages(response, args)
	if err != nil {
		var unknownErr unknownOutcomeError
		if errors.As(err, &unknownErr) {
			return nil, err
		}
		return nil, unknownOutcomeError{cause: fmt.Errorf("图片已经生成，但保存或预览准备失败：%w", err)}
	}
	return successfulToolResult(response, savedImages), nil
}

func successfulToolResult(response imageResponse, images []savedImage) map[string]interface{} {
	outputPaths := make([]string, 0, len(images))
	for _, saved := range images {
		outputPaths = append(outputPaths, saved.Path)
	}
	text := "图片已保存至: " + outputPaths[0]
	if len(outputPaths) > 1 {
		text = "图片已保存至:\n- " + strings.Join(outputPaths, "\n- ")
	}
	text += "\n以上路径是最终交付文件，请直接使用；除非用户明确要求，否则不要复制、移动或重命名。图片已包含在本次工具结果中，可直接在对话中显示。"
	content := make([]contentBlock, 0, len(images)+1)
	content = append(content, contentBlock{Type: "text", Text: text})
	for _, saved := range images {
		preview, mimeType := inlinePreview(saved.Data, saved.MimeType)
		content = append(content, contentBlock{
			Type:     "image",
			Data:     base64.StdEncoding.EncodeToString(preview),
			MimeType: mimeType,
		})
	}
	return map[string]interface{}{
		"content":           content,
		"structuredContent": savedResult{Model: response.Model, Created: response.Created, OutputPaths: outputPaths, Usage: response.Usage},
	}
}

type accountConfig struct {
	baseURL string
	apiKey  string
}

func loadAccountConfig() (accountConfig, error) {
	executable, err := os.Executable()
	if err != nil {
		return accountConfig{}, accountUnavailableError{}
	}
	return loadAccountConfigFromDataDir(filepath.Dir(executable))
}

func loadAccountConfigFromDataDir(dataDirectory string) (accountConfig, error) {
	settingsBytes, err := os.ReadFile(filepath.Join(dataDirectory, "settings.json"))
	if err != nil {
		return accountConfig{}, accountUnavailableError{}
	}
	var settings struct {
		CodexHome string `json:"codex_home"`
	}
	if json.Unmarshal(settingsBytes, &settings) != nil {
		return accountConfig{}, accountUnavailableError{}
	}
	home := strings.TrimSpace(settings.CodexHome)
	if home == "" || !filepath.IsAbs(home) {
		return accountConfig{}, accountUnavailableError{}
	}
	home = filepath.Clean(home)
	configBytes, err := os.ReadFile(filepath.Join(home, "config.toml"))
	if err != nil {
		return accountConfig{}, accountUnavailableError{}
	}
	configText := string(configBytes)
	activeProvider := tomlString(configText, "model_provider")
	section := providerSection(configText, activeProvider)
	baseURL := tomlString(section, "base_url")
	bearer := tomlString(section, "experimental_bearer_token")
	if baseURL == "" || bearer == "" {
		return accountConfig{}, accountUnavailableError{}
	}
	authBytes, err := os.ReadFile(filepath.Join(home, "auth.json"))
	if err != nil {
		return accountConfig{}, accountUnavailableError{}
	}
	var auth interface{}
	if json.Unmarshal(authBytes, &auth) != nil || findAPIKey(auth) != bearer {
		return accountConfig{}, accountUnavailableError{}
	}
	return accountConfig{baseURL: strings.TrimRight(baseURL, "/"), apiKey: bearer}, nil
}

func tomlString(text, key string) string {
	pattern := "(?m)^\\s*" + regexp.QuoteMeta(key) + "\\s*=\\s*[\"']([^\"']*)[\"']\\s*$"
	match := regexp.MustCompile(pattern).FindStringSubmatch(text)
	if len(match) == 2 {
		return strings.TrimSpace(match[1])
	}
	return ""
}

func providerSection(text, provider string) string {
	if provider == "" {
		return ""
	}
	target := "model_providers." + provider
	lines := strings.Split(strings.ReplaceAll(text, "\r\n", "\n"), "\n")
	var result []string
	inSection := false
	sectionPattern := regexp.MustCompile("^\\s*\\[([^\\]]+)\\]\\s*$")
	for _, line := range lines {
		match := sectionPattern.FindStringSubmatch(line)
		if len(match) == 2 {
			inSection = match[1] == target
			continue
		}
		if inSection {
			result = append(result, line)
		}
	}
	return strings.Join(result, "\n")
}

func findAPIKey(value interface{}) string {
	switch object := value.(type) {
	case map[string]interface{}:
		if key, ok := object["OPENAI_API_KEY"].(string); ok && strings.TrimSpace(key) != "" {
			return strings.TrimSpace(key)
		}
		for _, child := range object {
			if result := findAPIKey(child); result != "" {
				return result
			}
		}
	case []interface{}:
		for _, child := range object {
			if result := findAPIKey(child); result != "" {
				return result
			}
		}
	}
	return ""
}

func callGenerations(config accountConfig, args map[string]interface{}, prompt string) (imageResponse, error) {
	options, err := imageOptions(args, prompt)
	if err != nil {
		return imageResponse{}, err
	}
	count, err := generationCount(args)
	if err != nil {
		return imageResponse{}, err
	}
	options["n"] = count
	payload, _ := json.Marshal(options)
	request, err := http.NewRequest(http.MethodPost, config.baseURL+"/images/generations", bytes.NewReader(payload))
	if err != nil {
		return imageResponse{}, err
	}
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Authorization", "Bearer "+config.apiKey)
	return doImageRequest(request)
}

func callEdits(config accountConfig, args map[string]interface{}, prompt string, imageFiles []string) (imageResponse, error) {
	var body bytes.Buffer
	writer := multipart.NewWriter(&body)
	options, err := imageOptions(args, prompt)
	if err != nil {
		return imageResponse{}, err
	}
	for key, value := range options {
		if err := writer.WriteField(key, fmt.Sprint(value)); err != nil {
			return imageResponse{}, err
		}
	}
	field := "image"
	if len(imageFiles) > 1 {
		field = "image[]"
	}
	for _, imagePath := range imageFiles {
		file, err := os.Open(imagePath)
		if err != nil {
			return imageResponse{}, fmt.Errorf("找不到待修改图片：%s", imagePath)
		}
		part, err := writer.CreateFormFile(field, filepath.Base(imagePath))
		if err != nil {
			file.Close()
			return imageResponse{}, err
		}
		_, copyErr := io.Copy(part, file)
		file.Close()
		if copyErr != nil {
			return imageResponse{}, copyErr
		}
	}
	if maskPath := stringArg(args, "mask_path"); maskPath != "" {
		file, err := os.Open(maskPath)
		if err != nil {
			return imageResponse{}, fmt.Errorf("找不到蒙版图片：%s", maskPath)
		}
		part, err := writer.CreateFormFile("mask", filepath.Base(maskPath))
		if err != nil {
			file.Close()
			return imageResponse{}, err
		}
		_, copyErr := io.Copy(part, file)
		file.Close()
		if copyErr != nil {
			return imageResponse{}, copyErr
		}
	}
	if err := writer.Close(); err != nil {
		return imageResponse{}, err
	}
	request, err := http.NewRequest(http.MethodPost, config.baseURL+"/images/edits", &body)
	if err != nil {
		return imageResponse{}, err
	}
	request.Header.Set("Content-Type", writer.FormDataContentType())
	request.Header.Set("Authorization", "Bearer "+config.apiKey)
	return doImageRequest(request)
}

func imageOptions(args map[string]interface{}, prompt string) (map[string]interface{}, error) {
	options := map[string]interface{}{"model": imageModel, "prompt": prompt, "size": stringArgDefault(args, "size", defaultImageSize)}
	for _, key := range []string{"quality", "background", "output_format", "moderation", "input_fidelity", "response_format", "user"} {
		if value := stringArg(args, key); value != "" {
			options[key] = value
		}
	}
	if raw, ok := args["output_compression"]; ok && raw != nil {
		if value, isString := raw.(string); isString && strings.TrimSpace(value) == "" {
			return options, nil
		}
		compression, valid := integerValue(raw)
		if !valid || compression < 0 || compression > 100 {
			return nil, errors.New("output_compression 必须是 0 到 100 之间的整数")
		}
		options["output_compression"] = compression
	}
	return options, nil
}

func doImageRequest(request *http.Request) (imageResponse, error) {
	return doImageRequestWithClient(&http.Client{Timeout: requestTimeout}, request)
}

func doImageRequestWithClient(client *http.Client, request *http.Request) (imageResponse, error) {
	response, err := client.Do(request)
	if err != nil {
		return imageResponse{}, unknownOutcomeError{cause: err}
	}
	defer response.Body.Close()
	raw, err := io.ReadAll(response.Body)
	if err != nil {
		return imageResponse{}, unknownOutcomeError{cause: err}
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		var failure struct {
			Error struct {
				Message string `json:"message"`
			} `json:"error"`
			Message string `json:"message"`
		}
		_ = json.Unmarshal(raw, &failure)
		message := failure.Error.Message
		if message == "" {
			message = failure.Message
		}
		if message == "" {
			message = strings.TrimSpace(string(raw))
		}
		return imageResponse{}, fmt.Errorf("生图接口请求失败（%d）：%s", response.StatusCode, message)
	}
	var result imageResponse
	if err := json.Unmarshal(raw, &result); err != nil {
		return imageResponse{}, unknownOutcomeError{cause: errors.New("生图接口返回格式不正确")}
	}
	if len(result.Data) == 0 {
		return imageResponse{}, errors.New("生图接口响应中没有图片数据")
	}
	return result, nil
}

func saveImages(response imageResponse, args map[string]interface{}) ([]savedImage, error) {
	outputDirectory := defaultPicturesDirectory()
	if explicit := stringArg(args, "output_dir"); explicit != "" {
		home, _ := os.UserHomeDir()
		outputDirectory = absolutePath(expandHome(explicit, home))
	}
	if err := os.MkdirAll(outputDirectory, 0o755); err != nil {
		return nil, fmt.Errorf("无法创建图片目录：%w", err)
	}
	now := time.Now()
	stamp := now.Format("20060102_150405") + "_" + fmt.Sprintf("%03d", now.Nanosecond()/1e6)
	images := make([]savedImage, 0, len(response.Data))
	for index, item := range response.Data {
		buffer, mime, err := imageBytes(item)
		if err != nil {
			return nil, err
		}
		mimeType := imageMIMEType(buffer, mime, stringArg(args, "output_format"))
		extension := extensionForMIME(mimeType)
		suffix := ""
		if len(response.Data) > 1 {
			suffix = "_" + strconv.Itoa(index+1)
		}
		outputPath := filepath.Join(outputDirectory, "ChatGPT_"+stamp+suffix+extension)
		for collision := 1; fileExists(outputPath); collision++ {
			outputPath = filepath.Join(outputDirectory, fmt.Sprintf("ChatGPT_%s%s_%d%s", stamp, suffix, collision, extension))
		}
		if err := os.WriteFile(outputPath, buffer, 0o600); err != nil {
			return nil, fmt.Errorf("无法保存图片：%w", err)
		}
		images = append(images, savedImage{
			Path:     absolutePath(outputPath),
			Data:     buffer,
			MimeType: mimeType,
		})
	}
	return images, nil
}

func defaultPicturesDirectory() string {
	home, err := os.UserHomeDir()
	if err != nil {
		return "Pictures"
	}
	return filepath.Join(home, "Pictures")
}

func imageBytes(item imageItem) ([]byte, string, error) {
	if item.URL != "" {
		if strings.HasPrefix(item.URL, "data:") {
			return decodeDataURL(item.URL)
		}
		request, err := http.NewRequest(http.MethodGet, item.URL, nil)
		if err != nil {
			return nil, "", err
		}
		response, err := (&http.Client{Timeout: requestTimeout}).Do(request)
		if err != nil {
			return nil, "", unknownOutcomeError{cause: err}
		}
		defer response.Body.Close()
		if response.StatusCode < 200 || response.StatusCode >= 300 {
			return nil, "", fmt.Errorf("下载生成图片失败（%d）", response.StatusCode)
		}
		buffer, err := io.ReadAll(response.Body)
		if err != nil {
			return nil, "", unknownOutcomeError{cause: err}
		}
		return buffer, response.Header.Get("Content-Type"), nil
	}
	if item.Base64JSON != "" {
		if strings.HasPrefix(item.Base64JSON, "data:") {
			return decodeDataURL(item.Base64JSON)
		}
		buffer, err := base64.StdEncoding.DecodeString(strings.Join(strings.Fields(item.Base64JSON), ""))
		return buffer, "", err
	}
	return nil, "", errors.New("生图接口响应中没有 url 或 b64_json")
}

func decodeDataURL(value string) ([]byte, string, error) {
	comma := strings.Index(value, ",")
	if comma < 0 || !strings.Contains(value[:comma], ";base64") {
		return nil, "", errors.New("图片 data URL 格式不正确")
	}
	metadata := strings.TrimPrefix(value[:comma], "data:")
	mime := strings.Split(metadata, ";")[0]
	buffer, err := base64.StdEncoding.DecodeString(strings.Join(strings.Fields(value[comma+1:]), ""))
	return buffer, mime, err
}

func imageMIMEType(buffer []byte, mime, requested string) string {
	if len(buffer) >= 8 && bytes.Equal(buffer[:8], []byte{137, 80, 78, 71, 13, 10, 26, 10}) {
		return "image/png"
	}
	if len(buffer) >= 3 && bytes.Equal(buffer[:3], []byte{255, 216, 255}) {
		return "image/jpeg"
	}
	if len(buffer) >= 12 && string(buffer[:4]) == "RIFF" && string(buffer[8:12]) == "WEBP" {
		return "image/webp"
	}
	normalizedMIME := strings.ToLower(strings.TrimSpace(strings.Split(mime, ";")[0]))
	if normalizedMIME == "image/png" || normalizedMIME == "image/jpeg" || normalizedMIME == "image/webp" {
		return normalizedMIME
	}
	switch strings.ToLower(strings.TrimSpace(requested)) {
	case "jpeg", "jpg":
		return "image/jpeg"
	case "webp":
		return "image/webp"
	default:
		return "image/png"
	}
}

func extensionForMIME(mimeType string) string {
	switch mimeType {
	case "image/jpeg":
		return ".jpg"
	case "image/webp":
		return ".webp"
	default:
		return ".png"
	}
}

func inlinePreview(original []byte, mimeType string) ([]byte, string) {
	return inlinePreviewWithLimit(original, mimeType, inlineOriginalByteLimit)
}

func inlinePreviewWithLimit(original []byte, mimeType string, byteLimit int) ([]byte, string) {
	if len(original) <= byteLimit {
		return original, mimeType
	}
	source, _, err := image.Decode(bytes.NewReader(original))
	if err != nil {
		return original, mimeType
	}
	bounds := source.Bounds()
	width, height := bounds.Dx(), bounds.Dy()
	if width <= 0 || height <= 0 {
		return original, mimeType
	}
	previewWidth, previewHeight := scaledDimensions(width, height, previewMaxDimension)
	preview := image.NewRGBA(image.Rect(0, 0, previewWidth, previewHeight))
	draw.CatmullRom.Scale(preview, preview.Bounds(), source, bounds, draw.Over, nil)
	var output bytes.Buffer
	encoder := png.Encoder{CompressionLevel: png.BestSpeed}
	if err := encoder.Encode(&output, preview); err != nil {
		return original, mimeType
	}
	return output.Bytes(), "image/png"
}

func scaledDimensions(width, height, maximum int) (int, int) {
	if width <= maximum && height <= maximum {
		return width, height
	}
	if width >= height {
		return maximum, max(1, int(math.Round(float64(height)*float64(maximum)/float64(width))))
	}
	return max(1, int(math.Round(float64(width)*float64(maximum)/float64(height)))), maximum
}

func imagePaths(args map[string]interface{}) []string {
	paths := []string{}
	if value := stringArg(args, "image_path"); value != "" {
		paths = append(paths, value)
	}
	if values, ok := args["image_paths"].([]interface{}); ok {
		for _, value := range values {
			if item, ok := value.(string); ok && strings.TrimSpace(item) != "" {
				paths = append(paths, strings.TrimSpace(item))
			}
		}
	}
	unique := []string{}
	for _, item := range paths {
		found := false
		for _, existing := range unique {
			if existing == item {
				found = true
				break
			}
		}
		if !found {
			unique = append(unique, item)
		}
	}
	return unique
}

func stringArg(args map[string]interface{}, key string) string {
	value, ok := args[key].(string)
	if !ok {
		return ""
	}
	return strings.TrimSpace(value)
}

func stringArgDefault(args map[string]interface{}, key, fallback string) string {
	if value := stringArg(args, key); value != "" {
		return value
	}
	return fallback
}

func generationCount(args map[string]interface{}) (int, error) {
	value, ok := args["n"]
	if !ok || value == nil {
		return 1, nil
	}
	count, valid := integerValue(value)
	if !valid || count < 1 || count > 10 {
		return 0, errors.New("n 必须是 1 到 10 之间的整数")
	}
	return count, nil
}

func integerValue(value interface{}) (int, bool) {
	switch number := value.(type) {
	case float64:
		if math.IsNaN(number) || math.IsInf(number, 0) || math.Trunc(number) != number {
			return 0, false
		}
		return int(number), true
	case float32:
		converted := float64(number)
		if math.IsNaN(converted) || math.IsInf(converted, 0) || math.Trunc(converted) != converted {
			return 0, false
		}
		return int(number), true
	case int:
		return number, true
	case int8:
		return int(number), true
	case int16:
		return int(number), true
	case int32:
		return int(number), true
	case int64:
		return int(number), true
	case uint:
		return int(number), uint64(number) <= uint64(^uint(0)>>1)
	case uint8:
		return int(number), true
	case uint16:
		return int(number), true
	case uint32:
		return int(number), uint64(number) <= uint64(^uint(0)>>1)
	case uint64:
		return int(number), number <= uint64(^uint(0)>>1)
	case json.Number:
		parsed, err := strconv.ParseInt(string(number), 10, 64)
		if err != nil {
			return 0, false
		}
		return integerValue(parsed)
	default:
		return 0, false
	}
}

func expandHome(value, home string) string {
	if home != "" && (value == "~" || strings.HasPrefix(value, "~/") || strings.HasPrefix(value, "~\\")) {
		return filepath.Join(home, value[2:])
	}
	return value
}

func absolutePath(value string) string {
	if absolute, err := filepath.Abs(value); err == nil {
		return absolute
	}
	return value
}

func fileExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}
