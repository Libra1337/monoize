package main

import "testing"

func TestParseResolution(t *testing.T) {
	cases := []struct {
		raw      string
		width    int
		height   int
		wantFail bool
	}{
		{"1280x720", 1280, 720, false},
		{"1920X1080", 1920, 1080, false},
		{"720", 0, 0, true},
		{"32x32", 0, 0, true},
	}
	for _, tc := range cases {
		width, height, err := parseResolution(tc.raw)
		if tc.wantFail {
			if err == nil {
				t.Fatalf("parseResolution(%q) expected failure", tc.raw)
			}
			continue
		}
		if err != nil || width != tc.width || height != tc.height {
			t.Fatalf("parseResolution(%q) = %d,%d,%v; want %d,%d", tc.raw, width, height, err, tc.width, tc.height)
		}
	}
}

func TestSrtTime(t *testing.T) {
	if got := srtTime(3661.5); got != "01:01:01,500" {
		t.Fatalf("srtTime(3661.5) = %q", got)
	}
	if got := srtTime(-5); got != "00:00:00,000" {
		t.Fatalf("srtTime(-5) = %q", got)
	}
}
