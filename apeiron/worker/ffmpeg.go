package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
)

// runCommand executes a command capturing stderr for error surfaces (AP-W5).
func runCommand(ctx context.Context, name string, args ...string) (string, error) {
	command := exec.CommandContext(ctx, name, args...)
	var stderr bytes.Buffer
	command.Stderr = &stderr
	err := command.Run()
	if err != nil {
		tail := stderr.String()
		if len(tail) > 2048 {
			tail = tail[len(tail)-2048:]
		}
		if ctx.Err() != nil {
			return tail, ctx.Err()
		}
		return tail, fmt.Errorf("%s failed: %v: %s", name, err, tail)
	}
	return stderr.String(), nil
}

func probeDuration(ctx context.Context, cfg *config, path string) (float64, error) {
	output, err := exec.CommandContext(ctx, cfg.FFProbe, "-v", "error",
		"-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1",
		path).Output()
	if err != nil {
		return 0, err
	}
	return strconv.ParseFloat(strings.TrimSpace(string(output)), 64)
}

type assemblePayload struct {
	Clips []struct {
		AssetID     string  `json:"asset_id"`
		Kind        string  `json:"kind"`
		DurationSec float64 `json:"duration_secs"`
	} `json:"clips"`
	AudioAssetID    string `json:"audio_asset_id"`
	SubtitleAssetID string `json:"subtitle_asset_id"`
	Resolution      string `json:"resolution"`
	FPS             int    `json:"fps"`
}

// runAssemble normalizes every clip to one encode (H.264/AAC, resolution,
// fps), turns still images into timed segments, mixes the narration audio,
// and burns subtitles (AP-W5).
func runAssemble(ctx context.Context, c *client, cfg *config, j *job) error {
	var payload assemblePayload
	if err := json.Unmarshal(j.Payload, &payload); err != nil {
		return fmt.Errorf("parse payload: %w", err)
	}
	if len(payload.Clips) == 0 {
		return fmt.Errorf("assembly needs at least one clip")
	}
	resolution := payload.Resolution
	if resolution == "" {
		resolution = "1280x720"
	}
	width, height, err := parseResolution(resolution)
	if err != nil {
		return err
	}
	fps := payload.FPS
	if fps <= 0 {
		fps = 30
	}

	dir, err := workDir(j.ID)
	if err != nil {
		return err
	}
	defer os.RemoveAll(dir)

	var inputs []string
	var concatLines []string
	for index, clip := range payload.Clips {
		extension := "mp4"
		if clip.Kind == "image" {
			extension = "png"
		}
		source := filepath.Join(dir, fmt.Sprintf("in-%03d.%s", index, extension))
		if err := c.fetchFile(ctx, clip.AssetID, source); err != nil {
			return err
		}
		segment := filepath.Join(dir, fmt.Sprintf("seg-%03d.mp4", index))
		args := []string{"-y"}
		if clip.Kind == "image" {
			seconds := clip.DurationSec
			if seconds <= 0 {
				seconds = 4
			}
			if seconds > 30 {
				seconds = 30
			}
			args = append(args, "-loop", "1", "-t", fmt.Sprintf("%.3f", seconds), "-i", source)
		} else {
			args = append(args, "-i", source)
		}
		args = append(args,
			"-vf", fmt.Sprintf("scale=%d:%d:force_original_aspect_ratio=decrease,pad=%d:%d:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=%d", width, height, width, height, fps),
			"-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p",
			"-an", segment)
		if _, err := runCommand(ctx, cfg.FFMpeg, args...); err != nil {
			return fmt.Errorf("segment %d: %w", index, err)
		}
		inputs = append(inputs, segment)
		concatLines = append(concatLines, fmt.Sprintf("file '%s'", segment))
	}

	// Concat segments.
	concatList := filepath.Join(dir, "concat.txt")
	if err := os.WriteFile(concatList, []byte(strings.Join(concatLines, "\n")+"\n"), 0o644); err != nil {
		return err
	}
	concatenated := filepath.Join(dir, "joined.mp4")
	if _, err := runCommand(ctx, cfg.FFMpeg, "-y", "-f", "concat", "-safe", "0",
		"-i", concatList, "-c", "copy", concatenated); err != nil {
		return err
	}

	// Mix narration audio and burn subtitles.
	var audioPath, subtitlePath string
	if payload.AudioAssetID != "" {
		audioPath = filepath.Join(dir, "voice.mp3")
		if err := c.fetchFile(ctx, payload.AudioAssetID, audioPath); err != nil {
			return err
		}
	}
	if payload.SubtitleAssetID != "" {
		subtitlePath = filepath.Join(dir, "subs.srt")
		if err := c.fetchFile(ctx, payload.SubtitleAssetID, subtitlePath); err != nil {
			return err
		}
	}

	final := filepath.Join(dir, "final.mp4")
	args := []string{"-y", "-i", concatenated}
	if audioPath != "" {
		args = append(args, "-i", audioPath)
	}
	if audioPath != "" {
		args = append(args,
			"-map", "0:v:0", "-map", "1:a:0",
			"-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p",
			"-c:a", "aac", "-b:a", "160k", "-shortest",
		)
	} else {
		args = append(args, "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-an")
	}
	args = append(args, final)
	if _, err := runCommand(ctx, cfg.FFMpeg, args...); err != nil {
		return err
	}

	// Burn subtitles as a second pass when present (keeps the map logic
	// simple and the audio untouched).
	if subtitlePath != "" {
		burned := filepath.Join(dir, "burned.mp4")
		escaped := strings.ReplaceAll(subtitlePath, "\\", "/")
		escaped = strings.ReplaceAll(escaped, ":", "\\:")
		if _, err := runCommand(ctx, cfg.FFMpeg, "-y", "-i", final,
			"-vf", fmt.Sprintf("subtitles='%s'", escaped),
			"-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p",
			"-c:a", "copy", burned); err != nil {
			return err
		}
		final = burned
	}

	duration, _ := probeDuration(ctx, cfg, final)
	return c.uploadResult(ctx, j.ID, map[string]any{
		"extra": map[string]any{"duration_secs": duration},
	}, final, "video/mp4")
}

func parseResolution(resolution string) (int, int, error) {
	parts := strings.SplitN(strings.ToLower(resolution), "x", 2)
	if len(parts) != 2 {
		return 0, 0, fmt.Errorf("invalid resolution %q", resolution)
	}
	width, err := strconv.Atoi(strings.TrimSpace(parts[0]))
	if err != nil {
		return 0, 0, err
	}
	height, err := strconv.Atoi(strings.TrimSpace(parts[1]))
	if err != nil {
		return 0, 0, err
	}
	if width < 64 || height < 64 || width > 3840 || height > 3840 {
		return 0, 0, fmt.Errorf("resolution out of range: %q", resolution)
	}
	return width, height, nil
}
