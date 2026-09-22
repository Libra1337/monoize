package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
)

type ttsPayload struct {
	Text     string `json:"text"`
	Voice    string `json:"voice"`
	Speed    float64 `json:"speed"`
	Provider struct {
		BaseURL string `json:"base_url"`
		APIKey  string `json:"api_key"`
		Model   string `json:"model"`
	} `json:"provider"`
}

// runTTS synthesizes speech via an OpenAI-compatible /v1/audio/speech
// endpoint and uploads the mp3 back (AP-W4).
func runTTS(ctx context.Context, c *client, cfg *config, j *job) error {
	var payload ttsPayload
	if err := json.Unmarshal(j.Payload, &payload); err != nil {
		return fmt.Errorf("parse payload: %w", err)
	}
	if payload.Text == "" {
		return fmt.Errorf("tts text is empty")
	}
	model := payload.Provider.Model
	if model == "" {
		model = "tts-1"
	}
	voice := payload.Voice
	if voice == "" {
		voice = "alloy"
	}
	speed := payload.Speed
	if speed <= 0 {
		speed = 1.0
	}
	body, _ := json.Marshal(map[string]any{
		"model":           model,
		"input":           payload.Text,
		"voice":           voice,
		"speed":           speed,
		"response_format": "mp3",
	})
	request, err := http.NewRequestWithContext(ctx, http.MethodPost,
		payload.Provider.BaseURL+"/v1/audio/speech", bytes.NewReader(body))
	if err != nil {
		return err
	}
	request.Header.Set("Authorization", "Bearer "+payload.Provider.APIKey)
	request.Header.Set("Content-Type", "application/json")
	response, err := c.http.Do(request)
	if err != nil {
		return fmt.Errorf("tts upstream: %w", err)
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		raw, _ := io.ReadAll(io.LimitReader(response.Body, 4096))
		return fmt.Errorf("tts upstream status %d: %s", response.StatusCode, string(raw))
	}
	audio, err := io.ReadAll(io.LimitReader(response.Body, 200<<20))
	if err != nil {
		return err
	}
	if len(audio) == 0 {
		return fmt.Errorf("tts upstream returned empty audio")
	}
	dir, err := workDir(j.ID)
	if err != nil {
		return err
	}
	defer os.RemoveAll(dir)
	audioPath := filepath.Join(dir, "voice.mp3")
	if err := os.WriteFile(audioPath, audio, 0o644); err != nil {
		return err
	}
	return c.uploadResult(ctx, j.ID, map[string]any{
		"extra": map[string]any{"bytes": len(audio)},
	}, audioPath, "audio/mpeg")
}
