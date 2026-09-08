import { describe, expect, test } from 'bun:test'
import {
	nanoPerTokenToUsdPerMillion,
	normalizeMultiplier,
	usdPerMillionToNanoPerToken
} from '../src/lib/exact-decimal'

describe('exact decimal controls', () => {
	test('round-trips nano-token prices without binary floating point', () => {
		expect(usdPerMillionToNanoPerToken('1.001')).toBe('1001')
		expect(nanoPerTokenToUsdPerMillion('1001')).toBe('1.001')
		expect(usdPerMillionToNanoPerToken('0.0009')).toBe('0')
	})

	test('canonicalizes positive multipliers and rejects excess precision', () => {
		expect(normalizeMultiplier('01.230000000')).toBe('1.23')
		expect(normalizeMultiplier('1.0000000001')).toBeNull()
		expect(normalizeMultiplier('0')).toBeNull()
	})
})
