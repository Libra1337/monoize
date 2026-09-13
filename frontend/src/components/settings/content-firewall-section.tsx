import { useTranslation } from "react-i18next";

import {
  Field,
  FieldContent,
  FieldDescription,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import type { SystemSettings } from "@/lib/api";
import { SettingsGroup } from "./settings-category-panel";

interface ContentFirewallSectionProps {
  settings: SystemSettings;
  onChange: (updates: Partial<SystemSettings>) => void;
}

/**
 * Content firewall category (CF-19): the LLM judge configuration plus the
 * keyword hint list. The judge decides; keywords never block by themselves
 * (CF-2). See `spec/content-firewall.spec.md`.
 */
export function ContentFirewallSection({
  settings,
  onChange,
}: ContentFirewallSectionProps) {
  const { t } = useTranslation();

  const termCount = settings.moderation_blocked_words
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0).length;

  return (
    <div className="flex flex-col gap-8">
      <div className="grid gap-8 lg:grid-cols-2 lg:gap-12">
        <SettingsGroup label={t("settings.contentFirewallGroupSwitch")}>
          <Field orientation="horizontal">
            <FieldContent>
              <FieldLabel htmlFor="moderation_enabled">
                {t("settings.contentFirewallEnabled")}
              </FieldLabel>
              <FieldDescription>
                {t("settings.contentFirewallEnabledDescription")}
              </FieldDescription>
            </FieldContent>
            <Switch
              id="moderation_enabled"
              checked={settings.moderation_enabled}
              onCheckedChange={(checked) => onChange({ moderation_enabled: checked })}
            />
          </Field>
        </SettingsGroup>

        <SettingsGroup label={t("settings.contentFirewallGroupJudge")}>
          <Field orientation="horizontal">
            <FieldContent>
              <FieldLabel htmlFor="moderation_judge_enabled">
                {t("settings.contentFirewallJudgeEnabled")}
              </FieldLabel>
              <FieldDescription>
                {t("settings.contentFirewallJudgeEnabledDescription")}
              </FieldDescription>
            </FieldContent>
            <Switch
              id="moderation_judge_enabled"
              checked={settings.moderation_judge_enabled}
              onCheckedChange={(checked) => onChange({ moderation_judge_enabled: checked })}
            />
          </Field>
          <div className="grid gap-6 sm:grid-cols-2">
            <Field>
              <FieldLabel htmlFor="moderation_judge_base_url">
                {t("settings.contentFirewallJudgeBaseUrl")}
              </FieldLabel>
              <Input
                id="moderation_judge_base_url"
                className="font-mono text-xs"
                placeholder="https://api.example.com/v1"
                value={settings.moderation_judge_base_url}
                onChange={(e) => onChange({ moderation_judge_base_url: e.target.value })}
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="moderation_judge_model">
                {t("settings.contentFirewallJudgeModel")}
              </FieldLabel>
              <Input
                id="moderation_judge_model"
                className="font-mono text-xs"
                value={settings.moderation_judge_model}
                onChange={(e) => onChange({ moderation_judge_model: e.target.value })}
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="moderation_judge_api_key">
                {t("settings.contentFirewallJudgeApiKey")}
              </FieldLabel>
              <Input
                id="moderation_judge_api_key"
                type="password"
                className="font-mono text-xs"
                value={settings.moderation_judge_api_key}
                onChange={(e) => onChange({ moderation_judge_api_key: e.target.value })}
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="moderation_judge_timeout_ms">
                {t("settings.contentFirewallJudgeTimeout")}
              </FieldLabel>
              <Input
                id="moderation_judge_timeout_ms"
                type="number"
                min="1"
                value={settings.moderation_judge_timeout_ms}
                onChange={(e) =>
                  onChange({ moderation_judge_timeout_ms: parseInt(e.target.value) || 8000 })
                }
              />
              <FieldDescription>
                {t("settings.contentFirewallJudgeTimeoutDescription")}
              </FieldDescription>
            </Field>
          </div>
        </SettingsGroup>
      </div>

      <SettingsGroup label={t("settings.contentFirewallGroupWords")}>
        <Field>
          <FieldLabel htmlFor="moderation_blocked_words">
            {t("settings.contentFirewallWords")}
          </FieldLabel>
          <Textarea
            id="moderation_blocked_words"
            className="min-h-[180px] font-mono text-xs"
            spellCheck={false}
            value={settings.moderation_blocked_words}
            placeholder={t("settings.contentFirewallWordsPlaceholder")}
            onChange={(e) => onChange({ moderation_blocked_words: e.target.value })}
          />
          <FieldDescription>
            {t("settings.contentFirewallWordsDescription", { termCount })}
          </FieldDescription>
        </Field>
      </SettingsGroup>
    </div>
  );
}
