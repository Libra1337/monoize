// Apeiron render worker (AP-W1..W5): claims jobs from the Rust server and
// executes TTS synthesis, stock-material fetch, subtitle build, and ffmpeg
// assembly.
package main

import (
	"context"
	"log"
	"os"
	"os/signal"
	"sync"
	"syscall"
	"time"
)

func main() {
	cfg := loadConfig()
	client := newClient(cfg)

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	log.Printf("apeiron worker %s starting: server=%s ffmpeg=%s concurrency=%d kinds=%v",
		cfg.WorkerID, cfg.ServerURL, cfg.FFMpeg, cfg.Concurrency, cfg.Kinds)

	var wg sync.WaitGroup
	for i := 0; i < cfg.Concurrency; i++ {
		wg.Add(1)
		go func(slot int) {
			defer wg.Done()
			runLoop(ctx, client, cfg, slot)
		}(i)
	}
	wg.Wait()
	log.Printf("apeiron worker %s stopped", cfg.WorkerID)
}

func runLoop(ctx context.Context, client *client, cfg *config, slot int) {
	for {
		select {
		case <-ctx.Done():
			return
		default:
		}
		job, err := client.claim(ctx)
		if err != nil {
			log.Printf("[slot %d] claim error: %v", slot, err)
			sleepCtx(ctx, 3*time.Second)
			continue
		}
		if job == nil {
			sleepCtx(ctx, time.Duration(cfg.PollIntervalMS)*time.Millisecond)
			continue
		}
		log.Printf("[slot %d] job %s kind=%s claimed", slot, job.ID, job.Kind)
		execCtx, cancel := context.WithTimeout(ctx, time.Duration(cfg.JobTimeoutSec)*time.Second)
		var execErr error
		switch job.Kind {
		case "tts":
			execErr = runTTS(execCtx, client, cfg, job)
		case "material":
			execErr = runMaterial(execCtx, client, cfg, job)
		case "subtitle":
			execErr = runSubtitle(execCtx, client, cfg, job)
		case "assemble":
			execErr = runAssemble(execCtx, client, cfg, job)
		default:
			execErr = failJob(ctx, client, job, "unknown job kind "+job.Kind)
		}
		cancel()
		if execErr != nil {
			log.Printf("[slot %d] job %s failed: %v", slot, job.ID, execErr)
			if err := failJob(context.Background(), client, job, execErr.Error()); err != nil {
				log.Printf("[slot %d] fail report error: %v", slot, err)
			}
		} else {
			log.Printf("[slot %d] job %s done", slot, job.ID)
		}
	}
}

func sleepCtx(ctx context.Context, d time.Duration) {
	select {
	case <-ctx.Done():
	case <-time.After(d):
	}
}
