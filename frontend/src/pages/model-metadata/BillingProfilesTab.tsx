import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
	ArrowDown,
	ArrowUp,
	CheckCircle2,
	ChevronRight,
	CircleDollarSign,
	CloudDownload,
	Copy,
	PenLine,
	Plus,
	RefreshCw,
	Search,
	Settings2,
	Trash2
} from 'lucide-react'
import { toast } from 'sonner'
import { ModelBadge } from '@/components/ModelBadge'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import {
	copyPricingProfile,
	deleteBillingRateOptimistic,
	renamePricingProfileModel,
	syncModelMetadata,
	updatePricingProfilePatternsOptimistic,
	upsertBillingRateOptimistic,
	useBillingRateProfiles,
	useBillingRatesForProfile,
	useModelMetadata,
	usePricingProfilePatterns
} from '@/lib/swr'
import type { BillingRateRecord, PricingProfilePattern } from '@/lib/api'
import {
	formatNanoPerTokenPerMillion,
	nanoPerTokenToPerMillion,
	perMillionToNanoPerToken
} from '@/lib/exact-decimal'
import { cn } from '@/lib/utils'

type UsageClass = 'input_uncached' | 'cache_read' | 'output'

const visibleUsageClasses: Array<{ id: UsageClass; label: string }> = [
	{ id: 'input_uncached', label: 'Input' },
	{ id: 'cache_read', label: 'Cache read' },
	{ id: 'output', label: 'Output' }
]

/// UI17a: a rate renders under the symbol of its own stored currency.
function nanoToPerMillion(rate?: BillingRateRecord): string {
	return formatNanoPerTokenPerMillion(rate?.unit_price_nano, rate?.unit_price_currency)
}

/**
 * Prefills the CNY override form from an existing rate.
 *
 * Only a CNY-basis rate is prefilled. Copying a models.dev USD number into a CNY field would
 * keep the digits and change their meaning, turning a $3.00 list price into a ¥3.00 charge,
 * so a USD-basis rate leaves the field empty and the operator types the CNY price.
 */
function nanoToInput(rate?: BillingRateRecord): string {
	if (!rate || rate.unit_price_currency !== 'CNY') return ''
	return nanoPerTokenToPerMillion(rate.unit_price_nano) ?? ''
}

function nanoToPeakInput(rate?: BillingRateRecord): string {
	if (!rate || rate.unit_price_currency !== 'CNY' || !rate.peak_unit_price_nano) return ''
	return nanoPerTokenToPerMillion(rate.peak_unit_price_nano) ?? ''
}

function formatRatePrices(rate?: BillingRateRecord): { offPeak: string; peak: string | null } {
	return {
		offPeak: nanoToPerMillion(rate),
		peak: rate?.peak_unit_price_nano
			? formatNanoPerTokenPerMillion(rate.peak_unit_price_nano, rate.unit_price_currency)
			: null,
	}
}

function perMillionToNano(value: string): string {
	const converted = perMillionToNanoPerToken(value)
	if (converted == null) throw new Error('Price must be a non-negative decimal')
	return converted
}

function effectiveRate(rates: BillingRateRecord[], usageClass: UsageClass) {
	return rates
		.filter(rate => rate.enabled && rate.rate_kind === 'token' && rate.usage_class === usageClass)
		.sort((a, b) => {
			if (a.source === 'manual' && b.source !== 'manual') return -1
			if (b.source === 'manual' && a.source !== 'manual') return 1
			return b.priority - a.priority
		})[0]
}

function safeIdPart(value: string) {
	return value.toLowerCase().replace(/[^a-z0-9._-]+/g, '-')
}

export function BillingProfilesTab() {
	const { i18n } = useTranslation()
	const zh = i18n.language.startsWith('zh')
	const c = (zhText: string, enText: string) => zh ? zhText : enText
	const [selectedProfile, setSelectedProfile] = useState('')
	const { data: metadata = [], isLoading: metadataLoading } = useModelMetadata()
	// UI17c: profile names/counts come from the summary endpoint and rate rows load per
	// selected profile, so this tab never downloads the unfiltered rate catalog.
	const { data: profileSummaries = [], isLoading: ratesLoading } = useBillingRateProfiles()
	const { data: rates = [], isLoading: profileRatesLoading } = useBillingRatesForProfile(selectedProfile || null)
	const { data: patterns = [], isLoading: patternsLoading } = usePricingProfilePatterns()
	const [search, setSearch] = useState('')
	const [syncing, setSyncing] = useState(false)
	const [autoSyncError, setAutoSyncError] = useState<string | null>(null)
	const autoSyncAttempted = useRef(false)
	const [overrideTarget, setOverrideTarget] = useState<{ profile: string; model: string } | null>(null)
	const [overrideForm, setOverrideForm] = useState({
		input: '',
		inputPeak: '',
		cache: '',
		cachePeak: '',
		output: '',
		outputPeak: '',
	})
	const [savingOverride, setSavingOverride] = useState(false)
	const [copyTarget, setCopyTarget] = useState<string | null>(null)
	const [copyName, setCopyName] = useState('')
	const [copying, setCopying] = useState(false)
	const [renameTarget, setRenameTarget] = useState<string | null>(null)
	const [renameName, setRenameName] = useState('')
	const [renaming, setRenaming] = useState(false)
	const [patternDraft, setPatternDraft] = useState<PricingProfilePattern[]>([])
	const [patternsDirty, setPatternsDirty] = useState(false)
	const [savingPatterns, setSavingPatterns] = useState(false)

	// The summary rows carry source presence, so the first-run auto-sync check keeps its
	// old meaning without loading any rate row.
	const hasModelsDevRates = profileSummaries.some(summary => summary.has_models_dev)
	const profiles = useMemo(() => {
		const names = new Set<string>()
		for (const summary of profileSummaries) names.add(summary.pricing_profile)
		for (const item of metadata) if (item.models_dev_provider) names.add(item.models_dev_provider)
		return [...names].sort((a, b) => a.localeCompare(b))
	}, [metadata, profileSummaries])

	useEffect(() => {
		if (!profiles.length) return
		if (!selectedProfile || !profiles.includes(selectedProfile)) {
			setSelectedProfile(profiles.includes('openai') ? 'openai' : profiles[0])
		}
	}, [profiles, selectedProfile])

	useEffect(() => {
		if (!patternsDirty) setPatternDraft(patterns.map(pattern => ({ ...pattern })))
	}, [patterns, patternsDirty])

	// MB-A7: profile names must stay disjoint across account classes, so an operator who wants
	// the same prices for both classes copies the rate set under a second name.
	const runCopy = async () => {
		if (!copyTarget) return
		const target = copyName.trim()
		if (!target) return
		setCopying(true)
		try {
			const result = await copyPricingProfile(copyTarget, target)
			toast.success(c(`已复制 ${result.copied} 条费率到 ${result.target_profile}`, `Copied ${result.copied} rates to ${result.target_profile}`))
			setSelectedProfile(result.target_profile)
			setCopyTarget(null)
			setCopyName('')
		} catch (error) {
			toast.error(error instanceof Error ? error.message : c('复制失败', 'Copy failed'))
		} finally {
			setCopying(false)
		}
	}

	// MB-A9 / UI20a: a profile's model name is not fixed. UI20c requires reporting retained
	// synchronized rows, because those stay under the former name and are not a partial failure.
	const runRename = async () => {
		if (!renameTarget || !selectedProfile) return
		const target = renameName.trim()
		if (!target || target === renameTarget) return
		setRenaming(true)
		try {
			const result = await renamePricingProfileModel(selectedProfile, renameTarget, target)
			toast.success(c(`已把 ${result.written} 条费率改名为 ${result.target_model}`, `Renamed ${result.written} rates to ${result.target_model}`))
			if (result.synchronized_retained > 0) {
				toast.info(c(
					`${renameTarget} 仍保留 ${result.synchronized_retained} 条同步价格：同步费率由模型注册表拥有，不随改名移动。`,
					`${renameTarget} keeps ${result.synchronized_retained} synchronized prices. The model registry owns synchronized rates, so a rename does not move them.`
				))
			}
			setRenameTarget(null)
			setRenameName('')
		} catch (error) {
			toast.error(error instanceof Error ? error.message : c('改名失败', 'Rename failed'))
		} finally {
			setRenaming(false)
		}
	}

	const runSync = async (automatic = false) => {
		setSyncing(true)
		setAutoSyncError(null)
		try {
			const result = await syncModelMetadata()
			if (!automatic) toast.success(c(`已同步 ${result.upserted} 个模型`, `Synced ${result.upserted} models`))
		} catch (error) {
			const message = error instanceof Error ? error.message : c('models.dev 同步失败', 'models.dev sync failed')
			setAutoSyncError(message)
			if (!automatic) toast.error(message)
		} finally {
			setSyncing(false)
		}
	}

	useEffect(() => {
		if (metadataLoading || ratesLoading || autoSyncAttempted.current) return
		if (metadata.some(item => item.source === 'models_dev') || hasModelsDevRates) return
		autoSyncAttempted.current = true
		setSyncing(true)
		setAutoSyncError(null)
		void syncModelMetadata()
			.catch(error => setAutoSyncError(error instanceof Error ? error.message : 'models.dev sync failed'))
			.finally(() => setSyncing(false))
	}, [metadata, metadataLoading, hasModelsDevRates, ratesLoading])

	const selectedModelRates = useMemo(() => {
		const grouped = new Map<string, BillingRateRecord[]>()
		for (const rate of rates) {
			if (rate.pricing_profile !== selectedProfile || !rate.model_pattern) continue
			const list = grouped.get(rate.model_pattern) ?? []
			list.push(rate)
			grouped.set(rate.model_pattern, list)
		}
		for (const item of metadata) {
			if (item.models_dev_provider === selectedProfile && !grouped.has(item.model_id)) {
				grouped.set(item.model_id, [])
			}
		}
		return [...grouped.entries()]
			.filter(([model]) => model.toLowerCase().includes(search.trim().toLowerCase()))
			.sort(([a], [b]) => a.localeCompare(b))
	}, [metadata, rates, search, selectedProfile])

	const profileCounts = useMemo(() => {
		const counts = new Map<string, number>()
		for (const summary of profileSummaries) {
			counts.set(summary.pricing_profile, summary.model_count)
		}
		return counts
	}, [profileSummaries])

	const latestSync = useMemo(() => {
		const timestamps = metadata
			.filter(item => item.source === 'models_dev')
			.map(item => new Date(item.updated_at).getTime())
			.filter(Number.isFinite)
		return timestamps.length ? new Date(Math.max(...timestamps)) : null
	}, [metadata])

	const openOverride = (profile: string, model: string, modelRates: BillingRateRecord[]) => {
		setOverrideTarget({ profile, model })
		setOverrideForm({
			input: nanoToInput(effectiveRate(modelRates, 'input_uncached')),
			inputPeak: nanoToPeakInput(effectiveRate(modelRates, 'input_uncached')),
			cache: nanoToInput(effectiveRate(modelRates, 'cache_read')),
			cachePeak: nanoToPeakInput(effectiveRate(modelRates, 'cache_read')),
			output: nanoToInput(effectiveRate(modelRates, 'output')),
			outputPeak: nanoToPeakInput(effectiveRate(modelRates, 'output')),
		})
	}

		const saveOverride = async () => {
			if (!overrideTarget) return
			setSavingOverride(true)
			try {
				const values: Record<UsageClass, { offPeak: string; peak: string }> = {
					input_uncached: { offPeak: overrideForm.input, peak: overrideForm.inputPeak },
					cache_read: { offPeak: overrideForm.cache, peak: overrideForm.cachePeak },
					output: { offPeak: overrideForm.output, peak: overrideForm.outputPeak },
				}
				for (const { id: usageClass } of visibleUsageClasses) {
					const value = values[usageClass].offPeak
					const peakValue = values[usageClass].peak
					if (!value.trim() && usageClass === 'cache_read') {
						const existingManualCacheRate = rates.find(rate =>
							rate.source === 'manual' &&
							rate.pricing_profile === overrideTarget.profile &&
							rate.model_pattern === overrideTarget.model &&
							rate.usage_class === 'cache_read'
						)
						if (existingManualCacheRate) {
							await deleteBillingRateOptimistic(existingManualCacheRate.id)
						}
						continue
					}
					if (!value.trim()) throw new Error(c('输入和输出价格不能为空', 'Input and output prices are required'))
					const id = `manual:${safeIdPart(overrideTarget.profile)}:${safeIdPart(overrideTarget.model)}:${usageClass}`
					await upsertBillingRateOptimistic(id, {
						source: 'manual',
						pricing_profile: overrideTarget.profile,
						model_pattern: overrideTarget.model,
						provider_type: null,
						rate_kind: 'token',
						usage_class: usageClass,
						unit: 'token',
						unit_price_nano: perMillionToNano(value),
						// UI19c: this dialog is denominated in CNY, so the currency is stated on
						// every write instead of relying on the server default.
						unit_price_currency: 'CNY',
						// UI19d/MB-A8: a blank peak field clears the peak so the row always
						// bills at the off-peak price.
						peak_unit_price_nano: peakValue.trim() ? perMillionToNano(peakValue) : null,
						priority: 1000,
						enabled: true,
						match_json: {},
						raw_json: { editor: 'billing_profiles' }
					}, rates)
				}
				toast.success(c('手动价格已保存', 'Manual pricing saved'))
				setOverrideTarget(null)
			} catch (error) {
				toast.error(error instanceof Error ? error.message : c('保存失败', 'Save failed'))
			} finally {
				setSavingOverride(false)
			}
		}

	const deleteManualOverrides = async (modelRates: BillingRateRecord[]) => {
		const manual = modelRates.filter(rate => rate.source === 'manual')
		for (const rate of manual) await deleteBillingRateOptimistic(rate.id)
		toast.success(c('已恢复 models.dev 价格', 'Restored models.dev pricing'))
	}

	const savePatterns = async () => {
		if (patternDraft.some(pattern => !pattern.pattern.trim() || !pattern.pricing_profile.trim())) {
			toast.error(c('匹配规则不能为空', 'Match rules cannot be blank'))
			return
		}
		setSavingPatterns(true)
		try {
			await updatePricingProfilePatternsOptimistic(patternDraft, patterns)
			setPatternsDirty(false)
			toast.success(c('匹配规则已保存', 'Match rules saved'))
		} catch (error) {
			toast.error(error instanceof Error ? error.message : c('保存失败', 'Save failed'))
		} finally {
			setSavingPatterns(false)
		}
	}

	if ((metadataLoading || ratesLoading || patternsLoading) && !profiles.length) {
		return <div className='grid gap-4 lg:grid-cols-[280px_1fr]'><Skeleton className='h-[520px]' /><div className='flex flex-col gap-3'><Skeleton className='h-24' /><Skeleton className='h-[420px]' /></div></div>
	}

	return <>
		<div className='overflow-hidden rounded-xl border bg-card'>
			<div className='flex flex-col gap-3 border-b bg-muted/15 p-4 sm:flex-row sm:items-center sm:justify-between'>
				<div className='flex items-start gap-3'><div className='grid size-10 place-items-center rounded-lg bg-primary/10 text-primary'><CloudDownload className='size-5' /></div><div><div className='flex flex-wrap items-center gap-2'><h3 className='font-semibold'>models.dev</h3><Badge variant='outline' className='border-status-success/40 text-status-success'><CheckCircle2 className='mr-1 size-3' />{c('自动数据源', 'Automatic source')}</Badge></div><p className='mt-1 text-xs text-muted-foreground'>{latestSync ? c(`最近同步：${latestSync.toLocaleString()}`, `Last synced: ${latestSync.toLocaleString()}`) : c('尚未同步', 'Not synced yet')}</p></div></div>
				<Button variant='outline' onClick={() => void runSync()} disabled={syncing}><RefreshCw data-icon className={syncing ? 'animate-spin' : undefined} />{syncing ? c('同步中…', 'Syncing…') : c('立即同步', 'Sync now')}</Button>
			</div>
			{autoSyncError ? <div className='p-4 pb-0'><Alert variant='destructive'><AlertTitle>{c('自动同步失败', 'Automatic sync failed')}</AlertTitle><AlertDescription className='flex flex-wrap items-center justify-between gap-2'><span>{autoSyncError}</span><Button size='sm' variant='outline' onClick={() => void runSync()}>{c('重试', 'Retry')}</Button></AlertDescription></Alert></div> : null}

			<div className='lg:grid lg:min-h-[560px] lg:grid-cols-[280px_minmax(0,1fr)]'>
				<aside className='border-b bg-muted/10 lg:border-b-0 lg:border-r'>
					<div className='border-b px-4 py-3'><h4 className='text-sm font-medium'>{c('计费 Profile', 'Billing profiles')}</h4><p className='mt-1 text-xs text-muted-foreground'>{profiles.length} {c('个数据源', 'sources')}</p></div>
					<div className='flex gap-2 overflow-x-auto p-2 lg:flex-col lg:overflow-visible'>
						{profiles.map(profile => <button type='button' key={profile} onClick={() => setSelectedProfile(profile)} className={cn('flex min-w-40 shrink-0 items-center gap-3 rounded-lg border-l-2 px-3 py-2.5 text-left transition-colors lg:min-w-0', selectedProfile === profile ? 'border-l-primary bg-primary/10' : 'border-l-transparent hover:bg-muted')}><CircleDollarSign className='size-4 shrink-0 text-muted-foreground' /><span className='min-w-0 flex-1'><span className='block truncate text-sm font-medium'>{profile}</span><span className='block text-xs text-muted-foreground'>{profileCounts.get(profile) ?? 0} models</span></span><ChevronRight className='hidden size-4 text-muted-foreground lg:block' /></button>)}
					</div>
				</aside>

				<section className='min-w-0 p-4 sm:p-5'>
						<div className='flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between'><div><div className='flex flex-wrap items-center gap-2'><h3 className='text-lg font-semibold'>{selectedProfile || c('选择 Profile', 'Select a profile')}</h3><Badge variant='secondary'>{selectedModelRates.length} models</Badge>{selectedProfile ? <Button size='sm' variant='outline' onClick={() => { setCopyTarget(selectedProfile); setCopyName(`${selectedProfile}-copy`) }}><Copy data-icon />{c('复制为新 Profile', 'Copy to new profile')}</Button> : null}</div><p className='mt-1 text-sm text-muted-foreground'>{c('价格按每 100 万 tokens 显示，¥ 为 CNY，$ 为同步的 USD 价格；手动覆盖优先于同步价格。峰价仅在北京时间工作日 09:00–12:00 与 14:00–18:00 生效。', 'Prices are shown per 1M tokens. ¥ marks a CNY price, $ marks a synced USD price. Manual overrides take precedence. Peak prices apply Mon–Fri 09:00–12:00 and 14:00–18:00 Beijing time.')}</p></div><div className='relative w-full sm:w-72'><Search className='absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground' /><Input value={search} onChange={event => setSearch(event.target.value)} placeholder={c('搜索模型', 'Search models')} className='pl-9' /></div></div>

						<div className='mt-5 hidden grid-cols-[minmax(220px,1fr)_128px_128px_128px_90px] gap-2 border-b px-3 pb-2 text-xs font-medium text-muted-foreground md:grid'><span>Model</span><span>Input / 1M</span><span>Cache / 1M</span><span>Output / 1M</span><span /></div>
					<div className='mt-2 flex flex-col gap-2'>
						{profileRatesLoading && rates.length === 0 ? <>
							<Skeleton className='h-[76px] w-full rounded-lg' />
							<Skeleton className='h-[76px] w-full rounded-lg' />
							<Skeleton className='h-[76px] w-full rounded-lg' />
						</> : <>
						{selectedModelRates.map(([model, modelRates]) => {
							const manual = modelRates.some(rate => rate.source === 'manual')
							const metadataItem = metadata.find(item => item.model_id === model)
								return <div key={model} className='grid gap-3 rounded-lg border p-3 transition-colors hover:bg-muted/30 md:grid-cols-[minmax(220px,1fr)_128px_128px_128px_90px] md:items-center'>
									<div className='flex min-w-0 items-center gap-2'><ModelBadge model={model} provider={metadataItem?.models_dev_provider || selectedProfile} showDetails={false} /><div className='min-w-0'>{manual ? <Badge variant='default' className='mt-1'>{c('手动覆盖', 'Manual')}</Badge> : null}</div></div>
									{visibleUsageClasses.map(item => {
										const prices = formatRatePrices(effectiveRate(modelRates, item.id))
										return <div key={item.id} className='flex items-center justify-between gap-3 md:block'><span className='text-xs text-muted-foreground md:hidden'>{item.label}</span><span className='font-mono text-sm'>{prices.offPeak}{prices.peak ? <span className='mt-0.5 block text-xs text-muted-foreground'>{c(`峰 ${prices.peak}`, `peak ${prices.peak}`)}</span> : null}</span></div>
									})}
								<div className='flex justify-end gap-1'><Button size='sm' variant='ghost' onClick={() => openOverride(selectedProfile, model, modelRates)}>{c('编辑', 'Edit')}</Button><Button size='icon' variant='ghost' className='size-11 touch-manipulation sm:size-9' onClick={() => { setRenameTarget(model); setRenameName(model) }} aria-label={c('修改模型名', 'Rename model')}><PenLine data-icon /></Button>{manual ? <Button size='icon' variant='ghost' className='size-11 touch-manipulation sm:size-9' onClick={() => void deleteManualOverrides(modelRates)} aria-label={c('删除手动覆盖', 'Delete manual override')}><Trash2 data-icon /></Button> : null}</div>
							</div>
						})}
						{selectedModelRates.length === 0 ? <div className='rounded-lg border border-dashed p-10 text-center text-sm text-muted-foreground'>{c('这个 Profile 没有匹配的模型。', 'No models match this profile.')}</div> : null}
						</>}
					</div>

					<details className='group mt-6 rounded-xl border'>
						<summary className='flex cursor-pointer list-none items-center justify-between gap-3 p-4'><div className='flex items-center gap-3'><Settings2 className='size-4 text-muted-foreground' /><div><h4 className='text-sm font-medium'>{c('模型匹配规则', 'Model match rules')}</h4><p className='mt-0.5 text-xs text-muted-foreground'>{c('按顺序把请求模型映射到计费 Profile', 'Ordered rules map request models to billing profiles')}</p></div></div><ChevronRight className='size-4 transition-transform group-open:rotate-90' /></summary>
						<div className='flex flex-col gap-3 border-t p-4'>
							{patternDraft.map((pattern, index) => <div key={index} className='grid gap-2 sm:grid-cols-[44px_1fr_1fr_108px] sm:items-center'><span className='text-center font-mono text-xs text-muted-foreground'>{index + 1}</span><Input value={pattern.pattern} onChange={event => { setPatternDraft(previous => previous.map((item, itemIndex) => itemIndex === index ? { ...item, pattern: event.target.value } : item)); setPatternsDirty(true) }} placeholder='gpt-*' className='font-mono' /><Input value={pattern.pricing_profile} onChange={event => { setPatternDraft(previous => previous.map((item, itemIndex) => itemIndex === index ? { ...item, pricing_profile: event.target.value } : item)); setPatternsDirty(true) }} placeholder='openai' /><div className='flex items-center justify-end'><Button size='icon' variant='ghost' className='size-11 touch-manipulation sm:size-9' aria-label={c('上移规则', 'Move rule up')} disabled={index === 0} onClick={() => { const next = [...patternDraft]; [next[index - 1], next[index]] = [next[index], next[index - 1]]; setPatternDraft(next); setPatternsDirty(true) }}><ArrowUp data-icon /></Button><Button size='icon' variant='ghost' className='size-11 touch-manipulation sm:size-9' aria-label={c('下移规则', 'Move rule down')} disabled={index === patternDraft.length - 1} onClick={() => { const next = [...patternDraft]; [next[index + 1], next[index]] = [next[index], next[index + 1]]; setPatternDraft(next); setPatternsDirty(true) }}><ArrowDown data-icon /></Button><Button size='icon' variant='ghost' className='size-11 touch-manipulation sm:size-9' aria-label={c('删除规则', 'Delete rule')} onClick={() => { setPatternDraft(previous => previous.filter((_, itemIndex) => itemIndex !== index)); setPatternsDirty(true) }}><Trash2 data-icon /></Button></div></div>)}
							<div className='flex flex-wrap items-center justify-between gap-2'><Button variant='outline' size='sm' onClick={() => { setPatternDraft(previous => [...previous, { pattern: '', pricing_profile: selectedProfile }]); setPatternsDirty(true) }}><Plus data-icon />{c('添加规则', 'Add rule')}</Button><Button size='sm' disabled={!patternsDirty || savingPatterns} onClick={() => void savePatterns()}>{savingPatterns ? c('保存中…', 'Saving…') : c('保存规则', 'Save rules')}</Button></div>
						</div>
					</details>
				</section>
			</div>
		</div>

		<Dialog open={copyTarget !== null} onOpenChange={open => { if (!open) { setCopyTarget(null); setCopyName('') } }}>
			<DialogContent className='max-w-lg'><DialogHeader><DialogTitle>{c('复制 Profile', 'Copy profile')}</DialogTitle><DialogDescription>{copyTarget ?? ''}</DialogDescription></DialogHeader><div className='flex flex-col gap-4 py-2'><p className='text-sm text-muted-foreground'>{c('把这个 Profile 的所有费率复制到一个新名字。企业分组和普通分组不能共用同一个 Profile 名，所以两边同价需要两份副本。', 'Copies every rate of this profile under a new name. Enterprise and standard Groups cannot share a profile name, so matching prices need two copies.')}</p><div className='flex flex-col gap-2'><Label htmlFor='copy-profile-name'>{c('新 Profile 名', 'New profile name')}</Label><Input id='copy-profile-name' value={copyName} onChange={event => setCopyName(event.target.value)} placeholder='deepseek-std' /><p className='text-xs text-muted-foreground'>{c('目标 Profile 必须不存在任何费率，否则复制会被拒绝，以免覆盖正在计费的价格。', 'The target profile must have no rates. A non-empty target is refused so prices already billing traffic are never overwritten.')}</p></div></div><DialogFooter><Button variant='outline' onClick={() => { setCopyTarget(null); setCopyName('') }}>{c('取消', 'Cancel')}</Button><Button disabled={copying || !copyName.trim() || copyName.trim() === copyTarget} onClick={() => void runCopy()}>{copying ? c('复制中…', 'Copying…') : c('复制', 'Copy')}</Button></DialogFooter></DialogContent>
		</Dialog>

			<Dialog open={renameTarget !== null} onOpenChange={open => { if (!open) { setRenameTarget(null); setRenameName('') } }}>
				<DialogContent className='max-w-lg'><DialogHeader><DialogTitle>{c('修改模型名', 'Rename model')}</DialogTitle><DialogDescription>{renameTarget ? `${selectedProfile} / ${renameTarget}` : ''}</DialogDescription></DialogHeader><div className='flex flex-col gap-4 py-2'><p className='text-sm text-muted-foreground'>{c('把这个模型的所有费率移到一个新模型名下。上游改了模型名，或者要用第二个别名提供同样的价格时使用，不需要逐个 usage class 重新输入。', 'Moves every rate of this model to a new model name. Use it when an upstream model is renamed, or to serve the same prices under a second alias, without retyping each usage class.')}</p><div className='flex flex-col gap-2'><Label htmlFor='rename-model-name'>{c('新模型名', 'New model name')}</Label><Input id='rename-model-name' value={renameName} onChange={event => setRenameName(event.target.value)} className='font-mono' placeholder='claude-opus-5-eu' /><p className='text-xs text-muted-foreground'>{c('目标模型名在这个 Profile 里必须没有任何费率，否则改名会被拒绝，以免覆盖正在计费的价格。同步价格由模型注册表拥有，会保留在原名下。', 'The target model must have no rates in this profile. A non-empty target is refused so prices already billing traffic are never overwritten. Synchronized prices are owned by the model registry and stay under the former name.')}</p></div></div><DialogFooter><Button variant='outline' onClick={() => { setRenameTarget(null); setRenameName('') }}>{c('取消', 'Cancel')}</Button><Button disabled={renaming || !renameName.trim() || renameName.trim() === renameTarget} onClick={() => void runRename()}>{renaming ? c('改名中…', 'Renaming…') : c('确认改名', 'Rename')}</Button></DialogFooter></DialogContent>
			</Dialog>
			<Dialog open={!!overrideTarget} onOpenChange={open => { if (!open) setOverrideTarget(null) }}>
				<DialogContent className='max-w-2xl'><DialogHeader><DialogTitle>{c('手动价格覆盖', 'Manual price override')}</DialogTitle><DialogDescription>{overrideTarget ? `${overrideTarget.profile} / ${overrideTarget.model}` : ''}</DialogDescription></DialogHeader><div className='flex flex-col gap-4 py-2'><p className='text-sm text-muted-foreground'>{c('输入 CNY / 100 万 tokens。留空 Cache 表示不覆盖缓存价格。峰价仅在北京时间工作日 09:00–12:00 与 14:00–18:00 生效；留空峰价表示始终按谷价计费。', 'Enter CNY per 1M tokens. Leave cache blank to keep it unspecified. Peak prices apply Mon–Fri 09:00–12:00 and 14:00–18:00 Beijing time. Leave a peak field blank to always bill at the off-peak price.')}</p><div className='grid gap-4 sm:grid-cols-3'>{([
					{ key: 'input', peakKey: 'inputPeak', label: 'Input' },
					{ key: 'cache', peakKey: 'cachePeak', label: 'Cache read' },
					{ key: 'output', peakKey: 'outputPeak', label: 'Output' },
				] as const).map(item => <div key={item.key} className='flex flex-col gap-3 rounded-lg border p-3'><Label>{item.label}</Label><div className='flex flex-col gap-2'><Label className='text-xs text-muted-foreground'>{c('谷价', 'Off-peak')}</Label><div className='relative'><span className='absolute left-3 top-1/2 -translate-y-1/2 text-sm text-muted-foreground'>¥</span><Input type='text' inputMode='decimal' className='pl-7' value={overrideForm[item.key]} onChange={event => setOverrideForm(previous => ({ ...previous, [item.key]: event.target.value }))} /></div></div><div className='flex flex-col gap-2'><Label className='text-xs text-muted-foreground'>{c('峰价（可选）', 'Peak (optional)')}</Label><div className='relative'><span className='absolute left-3 top-1/2 -translate-y-1/2 text-sm text-muted-foreground'>¥</span><Input type='text' inputMode='decimal' className='pl-7' value={overrideForm[item.peakKey]} onChange={event => setOverrideForm(previous => ({ ...previous, [item.peakKey]: event.target.value }))} /></div></div></div>)}</div></div><DialogFooter><Button variant='outline' onClick={() => setOverrideTarget(null)}>{c('取消', 'Cancel')}</Button><Button disabled={savingOverride} onClick={() => void saveOverride()}>{savingOverride ? c('保存中…', 'Saving…') : c('保存覆盖', 'Save override')}</Button></DialogFooter></DialogContent>
			</Dialog>
	</>
}
