import { useTranslation } from "react-i18next";

import {
  Field,
  FieldContent,
  FieldDescription,
  FieldLabel,
} from "@/components/ui/field";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import type { SystemSettings } from "@/lib/api";
import { SettingsGroup } from "./settings-category-panel";

interface ContentFirewallSectionProps {
  settings: SystemSettings;
  onChange: (updates: Partial<SystemSettings>) => void;
}

/**
 * Content firewall category (CF-19): one switch plus one newline-separated
 * prohibited-term list. Matching runs before any upstream call; see
 * `spec/content-firewall.spec.md`.
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

      <SettingsGroup label={t("settings.contentFirewallGroupWords")}>
        <Field>
          <FieldLabel htmlFor="moderation_blocked_words">
            {t("settings.contentFirewallWords")}
          </FieldLabel>
          <Textarea
            id="moderation_blocked_words"
            className="min-h-[220px] font-mono text-xs"
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
