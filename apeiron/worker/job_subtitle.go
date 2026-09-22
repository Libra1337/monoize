package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

type subtitlePayload struct {
	Lines []struct {
		Text   string  `json:"text"`
		Weight float64 `json:"weight"`
	} `json:"lines"`
	AudioAssetID string `json:"audio_asset_id"`
}

// runSubtitle builds an SRT timed proportionally over the narration audio
// duration (ffprobe); without audio each cue lasts 3 s (AP-W4).
func runSubtitle(ctx context.Context, c *client, cfg *config, j *job) error {
	var payload subtitlePayload
	if err := json.Unmarshal(j.Payload, &payload); err != nil {
		return fmt.Errorf("parse payload: %w", err)
	}
	if len(payload.Lines) == 0 {
		return fmt.Errorf("no subtitle lines")
	}
	total := 3.0 * float64(len(payload.Lines))
	if payload.AudioAssetID != "" {
		dir, err := workDir(j.ID)
		if err != nil {
			return err
		}
		defer os.RemoveAll(dir)
		audioPath := filepath.Join(dir, "voice.mp3")
		if err := c.fetchFile(ctx, payload.AudioAssetID, audioPath); err != nil {
			return err
		}
		if seconds, err := probeDuration(ctx, cfg, audioPath); err == nil && seconds > 0 {
			total = seconds
		}
	}
	weightSum := 0.0
	for _, line := range payload.Lines {
		if line.Weight <= 0 {
			weightSum++
		} else {
			weightSum += line.Weight
		}
	}
	var builder strings.Builder
	cursor := 0.0
	for index, line := range payload.Lines {
		weight := line.Weight
		if weight <= 0 {
			weight = 1
		}
		duration := total * (weight / weightSum)
		start := cursor
		end := cursor + duration
		cursor = end
		fmt.Fprintf(&builder, "%d\n%s --> %s\n%s\n\n",
			index+1, srtTime(start), srtTime(end), strings.TrimSpace(line.Text))
	}
	srt := builder.String()
	return c.uploadResult(ctx, j.ID, map[string]any{
		"subtitle_srt": srt,
		"extra":        map[string]any{"total_duration_secs": total},
	}, "", "")
}

func srtTime(seconds float64) string {
	if seconds < 0 {
		seconds = 0
	}
	totalMs := int(seconds * 1000)
	ms := totalMs % 1000
	totalSec := totalMs / 1000
	sec := totalSec % 60
	totalMin := totalSec / 60
	min := totalMin % 60
	hour := totalMin / 60
	return fmt.Sprintf("%02d:%02d:%02d,%03d", hour, min, sec, ms)
}
