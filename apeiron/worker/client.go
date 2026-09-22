package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"mime/multipart"
	"net/http"
	"os"
	"path/filepath"
	"time"
)

type job struct {
	ID      string          `json:"id"`
	Kind    string          `json:"kind"`
	Payload json.RawMessage `json:"payload"`
	StepID  string          `json:"step_id"`
	RunID   string          `json:"run_id"`
}

type client struct {
	base  string
	token string
	id    string
	kinds []string
	http  *http.Client
}

func newClient(cfg *config) *client {
	return &client{
		base:  cfg.ServerURL,
		token: cfg.WorkerToken,
		id:    cfg.WorkerID,
		kinds: cfg.Kinds,
		http:  &http.Client{Timeout: 0},
	}
}

func (c *client) do(ctx context.Context, method, path string, body io.Reader, contentType string) ([]byte, int, error) {
	request, err := http.NewRequestWithContext(ctx, method, c.base+path, body)
	if err != nil {
		return nil, 0, err
	}
	request.Header.Set("Authorization", "Bearer "+c.token)
	if contentType != "" {
		request.Header.Set("Content-Type", contentType)
	}
	response, err := c.http.Do(request)
	if err != nil {
		return nil, 0, err
	}
	defer response.Body.Close()
	payload, err := io.ReadAll(io.LimitReader(response.Body, 512<<20))
	if err != nil {
		return nil, response.StatusCode, err
	}
	return payload, response.StatusCode, nil
}

// claim blocks until a job is available or returns (nil, nil) on empty queue.
func (c *client) claim(ctx context.Context) (*job, error) {
	body := map[string]any{"worker_id": c.id, "kinds": c.kinds}
	raw, _ := json.Marshal(body)
	payload, status, err := c.do(ctx, http.MethodPost, "/api/worker/claim", bytes.NewReader(raw), "application/json")
	if err != nil {
		return nil, err
	}
	if status != http.StatusOK {
		return nil, fmt.Errorf("claim status %d: %s", status, truncate(payload, 200))
	}
	var parsed struct {
		Job *job `json:"job"`
	}
	if err := json.Unmarshal(payload, &parsed); err != nil {
		return nil, err
	}
	return parsed.Job, nil
}

func (c *client) heartbeat(ctx context.Context, jobID string) {
	_, _, _ = c.do(ctx, http.MethodPost, "/api/worker/jobs/"+jobID+"/heartbeat", nil, "")
}

func failJob(ctx context.Context, c *client, j *job, message string) error {
	raw, _ := json.Marshal(map[string]string{"error": message})
	_, status, err := c.do(ctx, http.MethodPost, "/api/worker/jobs/"+j.ID+"/fail", bytes.NewReader(raw), "application/json")
	if err != nil {
		return err
	}
	if status != http.StatusOK {
		return fmt.Errorf("fail status %d", status)
	}
	return nil
}

// uploadResult posts multipart form: `result` JSON + optional `file`.
func (c *client) uploadResult(ctx context.Context, jobID string, result map[string]any, filePath string, fileMime string) error {
	var buffer bytes.Buffer
	writer := multipart.NewWriter(&buffer)
	resultRaw, _ := json.Marshal(result)
	if err := writer.WriteField("result", string(resultRaw)); err != nil {
		return err
	}
	if filePath != "" {
		file, err := os.Open(filePath)
		if err != nil {
			return err
		}
		defer file.Close()
		part, err := writer.CreateFormFile("file", filepath.Base(filePath))
		if err != nil {
			return err
		}
		if _, err := io.Copy(part, file); err != nil {
			return err
		}
	}
	if err := writer.Close(); err != nil {
		return err
	}
	payload, status, err := c.do(ctx, http.MethodPost, "/api/worker/jobs/"+jobID+"/result", bytes.NewReader(buffer.Bytes()), writer.FormDataContentType())
	if err != nil {
		return err
	}
	if status != http.StatusOK {
		return fmt.Errorf("result status %d: %s", status, truncate(payload, 200))
	}
	return nil
}

// fetchFile downloads a worker-scoped asset into dir with the given name.
func (c *client) fetchFile(ctx context.Context, assetID, destination string) error {
	payload, status, err := c.do(ctx, http.MethodGet, "/api/worker/files/"+assetID, nil, "")
	if err != nil {
		return err
	}
	if status != http.StatusOK {
		return fmt.Errorf("fetch asset %s status %d", assetID, status)
	}
	return os.WriteFile(destination, payload, 0o644)
}

// heartbeatLoop keeps the job lease alive during long ffmpeg runs (AP-W3).
func heartbeatLoop(ctx context.Context, c *client, jobID string) (stop func()) {
	inner, cancel := context.WithCancel(ctx)
	go func() {
		ticker := time.NewTicker(30 * time.Second)
		defer ticker.Stop()
		for {
			select {
			case <-inner.Done():
				return
			case <-ticker.C:
				c.heartbeat(inner, jobID)
			}
		}
	}()
	return cancel
}

func truncate(payload []byte, limit int) string {
	if len(payload) > limit {
		return string(payload[:limit])
	}
	return string(payload)
}
