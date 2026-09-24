import { useState } from "react";
import { createRoot } from "react-dom/client";
import { createInstance } from "i18next";
import { I18nextProvider } from "react-i18next";
import { TooltipProvider } from "../../src/components/ui/tooltip";
import { Composer, type ComposerMode } from "../../src/components/playground/composer";
import { usePlaygroundPrefs } from "../../src/components/playground/prefs";

const i18n = createInstance();
await i18n.init({ lng: "en", resources: {} });

export function Fixture() {
  const [text, setText] = useState("");
  const [mode, setMode] = useState<ComposerMode>("chat");
  const [prefs, setPref] = usePlaygroundPrefs();
  return (
    <div id="composer-width" style={{ width: 600 }}>
      <Composer
        mode={mode} onModeChange={setMode} text={text} onTextChange={setText}
        attachments={[]} onAddFiles={() => {}} onRemoveAttachment={() => {}}
        onSend={() => {}} onStop={() => {}} canSend={false} isBusy={false}
        blockedHint={null} prefs={prefs} setPref={setPref} groups={[]}
        groupsLoading={false} models={[]} modelsLoading={false} apiKeys={[]}
        keysLoading={false} resolvedKeyId={null} isDraggingFiles={false}
      />
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <I18nextProvider i18n={i18n}>
    <TooltipProvider><Fixture /></TooltipProvider>
  </I18nextProvider>,
);
