package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
)

type materialPayload struct {
	Query       string  `json:"query"`
	URL         string  `json:"url"`
	DurationSec float64 `json:"duration_secs"`
	Provider    *struct {
		Source string `json:"source"`
		APIKey string `json:"api_key"`
	} `json:"provider"`
}

// runMaterial fetches one stock clip: either a direct URL download or a
// Pexels API search by keywords (AP-W4).
func runMaterial(ctx context.Context, c *client, cfg *config, j *job) error {
	var payload materialPayload
	if err := json.Unmarshal(j.Payload, &payload); err != nil {
		return fmt.Errorf("parse payload: %w", err)
	}
	var videoURL string
	if payload.URL != "" {
		videoURL = payload.URL
	} else {
		if payload.Provider == nil || payload.Provider.APIKey == "" {
			return fmt.Errorf("no material provider and no direct url")
		}
		found, err := searchPexels(ctx, payload.Provider.APIKey, payload.Query, 1)
		if err != nil {
			return err
		}
		if len(found) == 0 {
			return fmt.Errorf("no stock result for query %q", payload.Query)
		}
		videoURL = found[0]
	}
	dir, err := workDir(j.ID)
	if err != nil {
		return err
	}
	defer os.RemoveAll(dir)
	clipPath := filepath.Join(dir, "clip.mp4")
	if err := downloadTo(ctx, videoURL, clipPath, 200<<20); err != nil {
		return err
	}
	return c.uploadResult(ctx, j.ID, map[string]any{
		"extra": map[string]any{"source_url": videoURL},
	}, clipPath, "video/mp4")
}

func searchPexels(ctx context.Context, apiKey, query string, limit int) ([]string, error) {
	endpoint := fmt.Sprintf(
		"https://api.pexels.com/videos/search?per_page=%d&orientation=landscape&query=%s",
		limit, url.QueryEscape(query))
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint, nil)
	if err != nil {
		return nil, err
	}
	request.Header.Set("Authorization", apiKey)
	response, err := http.DefaultClient.Do(request)
	if err != nil {
		return nil, fmt.Errorf("pexels search: %w", err)
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("pexels search status %d", response.StatusCode)
	}
	var parsed struct {
		Videos []struct {
			VideoFiles []struct {
				Link  string `json:"link"`
				Width int    `json:"width"`
			} `json:"video_files"`
		} `json:"videos"`
	}
	if err := json.NewDecoder(io.LimitReader(response.Body, 8<<20)).Decode(&parsed); err != nil {
		return nil, fmt.Errorf("decode pexels response: %w", err)
	}
	// Prefer an HD-ish landscape file; fall back to the first link.
	var links []string
	for _, video := range parsed.Videos {
		best := ""
		for _, file := range video.VideoFiles {
			if best == "" || (file.Width >= 640 && file.Width <= 1920) {
				best = file.Link
			}
		}
		if best != "" {
			links = append(links, best)
		}
	}
	return links, nil
}

func downloadTo(ctx context.Context, rawURL, destination string, limit int64) error {
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, rawURL, nil)
	if err != nil {
		return err
	}
	response, err := http.DefaultClient.Do(request)
	if err != nil {
		return fmt.Errorf("download: %w", err)
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return fmt.Errorf("download status %d", response.StatusCode)
	}
	file, err := os.Create(destination)
	if err != nil {
		return err
	}
	defer file.Close()
	written, err := io.Copy(file, io.LimitReader(response.Body, limit))
	if err != nil {
		return fmt.Errorf("download copy: %w", err)
	}
	if written == 0 {
		return fmt.Errorf("downloaded file is empty")
	}
	return nil
}
