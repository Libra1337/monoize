import { describe, expect, test } from 'bun:test'
import {
	formatNanoPerTokenPerMillion,
	nanoPerTokenToPerMillion,
	normalizeMultiplier,
	perMillionToNanoPerToken,
	readRateCurrency
} from '../src/lib/exact-decimal'

describe('exact decimal controls', () => {
	test('round-trips nano-token prices without binary floating point', () => {
		expect(perMillionToNanoPerToken('1.001')).toBe('1001')
		expect(nanoPerTokenToPerMillion('1001')).toBe('1.001')
		expect(perMillionToNanoPerToken('0.0009')).toBe('0')
	})

	// UI17a: a price renders under the symbol of its own currency, and a breakdown written
	// before migration 066 carries no currency field, so it must read as USD.
	test('renders a per-million price under its own currency symbol', () => {
		expect(formatNanoPerTokenPerMillion('1001', 'CNY')).toBe('¥1.001')
		expect(formatNanoPerTokenPerMillion('1001', 'USD')).toBe('$1.001')
		expect(formatNanoPerTokenPerMillion('1001')).toBe('$1.001')
		expect(formatNanoPerTokenPerMillion(null, 'CNY')).toBe('—')
		expect(readRateCurrency('CNY')).toBe('CNY')
		expect(readRateCurrency(undefined)).toBe('USD')
		expect(readRateCurrency('EUR')).toBe('USD')
	})

	test('canonicalizes positive multipliers and rejects excess precision', () => {
		expect(normalizeMultiplier('01.230000000')).toBe('1.23')
		expect(normalizeMultiplier('1.0000000001')).toBeNull()
		expect(normalizeMultiplier('0')).toBeNull()
	})
})
