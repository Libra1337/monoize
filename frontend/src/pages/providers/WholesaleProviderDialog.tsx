import { useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import useSWR from 'swr'
import { toast } from 'sonner'
import { Loader2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
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
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectTrigger,
	SelectValue
} from '@/components/ui/select'
import { Skeleton } from '@/components/ui/skeleton'
import { GroupSingleSelect } from '@/components/groups/GroupPicker'
import { api } from '@/lib/api'
import { normalizeMultiplier } from '@/lib/exact-decimal'
import { createWholesaleProviderOptimistic, useDashboardGroups, useProviders } from '@/lib/swr'

interface WholesaleProviderDialogProps {
	open: boolean
	onOpenChange: (open: boolean) => void
}

/**
 * PP-WF2: creation form for a wholesale Provider in an agent Group. The source Provider
 * supplies the upstream configuration and the default per-model multipliers; every
 * multiplier is editable before the copy is materialized (PP-W5).
 */
export function WholesaleProviderDialog({ open, onOpenChange }: WholesaleProviderDialogProps) {
	const { i18n, t } = useTranslation()
	const zh = i18n.language.startsWith('zh')
	const c = (zhText: string, enText: string) => zh ? zhText : enText

	const { data: providers = [], isLoading: providersLoading } = useProviders()
	const { data: groups = [], isLoading: groupsLoading } = useDashboardGroups()
	const agentGroups = useMemo(
		() => groups.filter(group => group.account_class === 'agent'),
		[groups]
	)
	const groupById = useMemo(() => new Map(groups.map(group => [group.id, group])), [groups])
	const retailProviders = useMemo(
		() => providers.filter(provider => groupById.get(provider.group_id)?.account_class !== 'agent'),
		[groupById, providers]
	)

	const [groupId, setGroupId] = useState('')
	const [sourceProviderId, setSourceProviderId] = useState('')
	const [name, setName] = useState('')
	const [channelName, setChannelName] = useState('')
	const [multiplier, setMultiplier] = useState('')
	const [modelMultipliers, setModelMultipliers] = useState<Record<string, string>>({})
	const [exposureConfirmed, setExposureConfirmed] = useState(false)
	const [submitting, setSubmitting] = useState(false)

	const { data: source, isLoading: sourceLoading } = useSWR(
		open && sourceProviderId ? ['wholesale-source', sourceProviderId] : null,
		() => api.getProvider(sourceProviderId)
	)

	// PP-W5: prefill the copy with the source's effective values whenever the source changes.
	useEffect(() => {
		if (!source) return
		setName(source.name)
		setChannelName(source.channel.name)
		setMultiplier(source.multiplier)
		const next: Record<string, string> = {}
		for (const [model, entry] of Object.entries(source.channel.models)) {
			next[model] = entry.multiplier_override ?? source.multiplier
		}
		setModelMultipliers(next)
	}, [source])

	const models = useMemo(
		() => Object.keys(source?.channel.models ?? {}).sort((left, right) => left.localeCompare(right)),
		[source]
	)

	const valid =
		!!groupId &&
		!!sourceProviderId &&
		exposureConfirmed &&
		models.length > 0 &&
		models.every(model => normalizeMultiplier(modelMultipliers[model] ?? '') !== null) &&
		(normalizeMultiplier(multiplier) !== null)

	const handleSubmit = async () => {
		if (!valid || submitting) return
		setSubmitting(true)
		try {
			await createWholesaleProviderOptimistic({
				group_id: groupId,
				source_provider_id: sourceProviderId,
				name: name.trim() || undefined,
				channel_name: channelName.trim() || undefined,
				multiplier: normalizeMultiplier(multiplier) ?? undefined,
				model_multipliers: Object.fromEntries(
					models
						.map(model => [model, normalizeMultiplier(modelMultipliers[model] ?? '')])
						.filter((entry): entry is [string, string] => entry[1] !== null)
				),
				confirm_public_exposure: true
			})
			toast.success(c('批发 Provider 已创建', 'Wholesale provider created'))
			onOpenChange(false)
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('common.error'))
		} finally {
			setSubmitting(false)
		}
	}

	const reset = (nextOpen: boolean) => {
		if (!nextOpen) {
			setSourceProviderId('')
			setName('')
			setChannelName('')
			setMultiplier('')
			setModelMultipliers({})
			setExposureConfirmed(false)
		}
		onOpenChange(nextOpen)
	}

	return (
		<Dialog open={open} onOpenChange={reset}>
			<DialogContent className='max-h-[85vh] overflow-y-auto sm:max-w-2xl'>
				<DialogHeader>
					<DialogTitle>{c('新建批发 Provider', 'Add wholesale provider')}</DialogTitle>
					<DialogDescription>
						{c('选择现有分组内的 Provider，复制其上游配置，并指定代理分组的批发倍率。',
							'Pick an existing provider, copy its upstream configuration, and set the wholesale multipliers for the agent group.')}
					</DialogDescription>
				</DialogHeader>

				<div className='grid gap-4'>
					<div className='grid gap-2'>
						<Label>{c('代理分组', 'Agent group')}</Label>
						<GroupSingleSelect
							value={groupId}
							groups={agentGroups}
							loading={groupsLoading}
							onChange={setGroupId}
						/>
						<p className='text-xs text-muted-foreground'>
							{c('批发 Provider 只能创建在代理分组内。', 'Wholesale providers can only be created inside an agent group.')}
						</p>
					</div>

					<div className='grid gap-2'>
						<Label>{c('源 Provider', 'Source provider')}</Label>
						<Select
							value={sourceProviderId}
							onValueChange={setSourceProviderId}
							disabled={providersLoading}
						>
							<SelectTrigger><SelectValue placeholder={providersLoading ? '…' : c('选择源 Provider', 'Select a source provider')} /></SelectTrigger>
							<SelectContent>
								<SelectGroup>
									{retailProviders.map(provider => (
										<SelectItem key={provider.id} value={provider.id}>
											{provider.name} · {groupById.get(provider.group_id)?.name ?? provider.group_id}
										</SelectItem>
									))}
								</SelectGroup>
							</SelectContent>
						</Select>
						<p className='text-xs text-muted-foreground'>
							{c('仅列出标准、企业、私有分组内的 Provider。', 'Only providers from the standard, enterprise, and private groups are listed.')}
						</p>
					</div>

					{sourceLoading && (
						<div className='space-y-2'>
							<Skeleton className='h-9 w-full' />
							<Skeleton className='h-9 w-full' />
						</div>
					)}

					{source && !sourceLoading && (
						<>
							<div className='grid gap-4 sm:grid-cols-2'>
								<div className='grid gap-2'>
									<Label>{c('名称', 'Name')}</Label>
									<Input value={name} onChange={event => setName(event.target.value)} />
								</div>
								<div className='grid gap-2'>
									<Label>{c('默认倍率', 'Default multiplier')}</Label>
									<Input
										type='text'
										inputMode='decimal'
										value={multiplier}
										onChange={event => setMultiplier(event.target.value)}
									/>
								</div>
							</div>

							<div className='grid gap-2'>
								<div className='flex items-center justify-between'>
									<Label>{c('模型倍率', 'Model multipliers')}</Label>
									<span className='text-xs text-muted-foreground'>
										{c('默认继承源 Provider 的有效倍率，可单独修改。', 'Defaults inherit the source effective multipliers and are editable per model.')}
									</span>
								</div>
								<div className='max-h-56 overflow-y-auto rounded-md border'>
									{models.map(model => (
										<div key={model} className='flex items-center gap-3 border-b px-3 py-2 last:border-b-0'>
											<span className='min-w-0 flex-1 truncate font-mono text-xs'>{model}</span>
											<Input
												aria-label={`multiplier-${model}`}
												className='h-8 w-32 shrink-0 font-mono text-xs'
												type='text'
												inputMode='decimal'
												value={modelMultipliers[model] ?? ''}
												onChange={event => setModelMultipliers(previous => ({ ...previous, [model]: event.target.value }))}
											/>
										</div>
									))}
								</div>
							</div>

							<div className='flex items-start gap-3 rounded-md border p-3'>
								<Checkbox
									id='wholesale-public-exposure'
									checked={exposureConfirmed}
									onCheckedChange={checked => setExposureConfirmed(checked === true)}
								/>
								<Label htmlFor='wholesale-public-exposure' className='text-sm font-normal leading-5'>
									{t('groups.providerPublicExposureConfirm')}
								</Label>
							</div>
						</>
					)}
				</div>

				<DialogFooter>
					<Button variant='outline' onClick={() => reset(false)}>{t('common.cancel')}</Button>
					<Button onClick={handleSubmit} disabled={!valid || submitting}>
						{submitting && <Loader2 className='mr-2 h-4 w-4 animate-spin' />}
						{c('创建批发 Provider', 'Create wholesale provider')}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	)
}
