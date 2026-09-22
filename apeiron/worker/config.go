package main

import (
	"fmt"
	"os"
	"strings"
)

type config struct {
	ServerURL      string
	WorkerToken    string
	WorkerID       string
	FFMpeg         string
	FFProbe        string
	Concurrency    int
	PollIntervalMS int
	JobTimeoutSec  int
	Kinds          []string
}

func envOr(name, fallback string) string {
	if value := strings.TrimSpace(os.Getenv(name)); value != "" {
		return value
	}
	return fallback
}

func loadConfig() *config {
	kinds := strings.Split(envOr("WORKER_KINDS", "tts,material,subtitle,assemble"), ",")
	cleaned := make([]string, 0, len(kinds))
	for _, kind := range kinds {
		if kind = strings.TrimSpace(kind); kind != "" {
			cleaned = append(cleaned, kind)
		}
	}
	hostname, _ := os.Hostname()
	concurrency := envInt("WORKER_CONCURRENCY", 1)
	return &config{
		ServerURL:      strings.TrimRight(envOr("APEIRON_SERVER_URL", "http://127.0.0.1:8090"), "/"),
		WorkerToken:    envOr("APEIRON_WORKER_TOKEN", ""),
		WorkerID:       envOr("WORKER_ID", fmt.Sprintf("worker-%s", hostname)),
		FFMpeg:         envOr("FFMPEG_PATH", "ffmpeg"),
		FFProbe:        envOr("FFPROBE_PATH", "ffprobe"),
		Concurrency:    concurrency,
		PollIntervalMS: envInt("WORKER_POLL_INTERVAL_MS", 1500),
		JobTimeoutSec:  envInt("WORKER_JOB_TIMEOUT_SEC", 1500),
		Kinds:          cleaned,
	}
}

func envInt(name string, fallback int) int {
	raw := strings.TrimSpace(os.Getenv(name))
	if raw == "" {
		return fallback
	}
	var value int
	if _, err := fmt.Sscanf(raw, "%d", &value); err != nil || value <= 0 {
		return fallback
	}
	return value
}

// workDir returns a fresh scratch directory for one job.
func workDir(jobID string) (string, error) {
	dir := fmt.Sprintf("%s/apeiron-%s", os.TempDir(), sanitize(jobID))
	if err := os.RemoveAll(dir); err != nil {
		return "", err
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return "", err
	}
	return dir, nil
}

func sanitize(raw string) string {
	var builder strings.Builder
	for _, ch := range raw {
		if (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || (ch >= '0' && ch <= '9') || ch == '-' {
			builder.WriteRune(ch)
		} else {
			builder.WriteRune('_')
		}
	}
	return builder.String()
}
