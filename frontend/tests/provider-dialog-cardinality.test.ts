import { readFileSync } from 'node:fs'
import { describe, expect, test } from 'bun:test'

const source = readFileSync(
	new URL('../src/pages/providers/ProviderDialog.tsx', import.meta.url),
	'utf8'
)

describe('Provider editor cardinality', () => {
	test('uses a singular Group selector', () => {
		expect(source).toContain('GroupSingleSelect')
		expect(source).not.toContain('GroupMultiSelect')
	})

	test('does not expose Channel creation, duplication, or deletion', () => {
		expect(source).not.toContain('addChannel')
		expect(source).not.toContain('duplicateChannel')
		expect(source).not.toContain('removeChannel')
	})
})
