export type ApiSampleLanguage = "curl" | "python" | "javascript" | "go";
export type ApiFamily = "responses" | "chat" | "messages" | "gemini" | "images";

export const API_SAMPLE_LANGUAGES: ApiSampleLanguage[] = [
  "curl",
  "python",
  "javascript",
  "go",
];

export const API_FAMILIES: ApiFamily[] = [
  "responses",
  "chat",
  "messages",
  "gemini",
  "images",
];

interface ApiFamilyDefinition {
  method: "POST";
  path: string;
  body: Record<string, unknown>;
  supportsStreaming: boolean;
  successShape: string;
  commonErrorShape: string;
}

const COMMON_ERROR_SHAPE = `{
  "error": {
    "code": "invalid_request",
    "message": "Request validation failed"
  }
}`;

const DEFINITIONS: Record<ApiFamily, ApiFamilyDefinition> = {
  responses: {
    method: "POST",
    path: "/responses",
    body: { model: "gpt-5", input: "Explain vector databases." },
    supportsStreaming: true,
    successShape: `{
  "id": "resp_...",
  "object": "response",
  "output": []
}`,
    commonErrorShape: COMMON_ERROR_SHAPE,
  },
  chat: {
    method: "POST",
    path: "/chat/completions",
    body: {
      model: "gpt-5",
      messages: [{ role: "user", content: "Hello" }],
      stream: true,
    },
    supportsStreaming: true,
    successShape: `{
  "id": "chatcmpl_...",
  "object": "chat.completion",
  "choices": []
}`,
    commonErrorShape: COMMON_ERROR_SHAPE,
  },
  messages: {
    method: "POST",
    path: "/messages",
    body: {
      model: "claude-sonnet-4",
      max_tokens: 1024,
      messages: [{ role: "user", content: "Hello" }],
    },
    supportsStreaming: true,
    successShape: `{
  "id": "msg_...",
  "type": "message",
  "content": []
}`,
    commonErrorShape: COMMON_ERROR_SHAPE,
  },
  gemini: {
    method: "POST",
    path: "/responses",
    body: { model: "gemini-2.5-pro", input: "Hello" },
    supportsStreaming: true,
    successShape: `{
  "id": "resp_...",
  "object": "response",
  "output": []
}`,
    commonErrorShape: COMMON_ERROR_SHAPE,
  },
  images: {
    method: "POST",
    path: "/images/generations",
    body: { model: "gpt-image-1", prompt: "A blue paper console" },
    supportsStreaming: false,
    successShape: `{
  "id": "image_...",
  "created": 0,
  "data": []
}`,
    commonErrorShape: COMMON_ERROR_SHAPE,
  },
};

export function apiFamilyDefinition(family: ApiFamily): ApiFamilyDefinition {
  return DEFINITIONS[family];
}

function normalizedBaseUrl(baseUrl: string): string {
  return baseUrl.replace(/\/+$/, "");
}

function pythonLiteral(value: unknown, depth = 0): string {
  const indent = "    ".repeat(depth);
  const childIndent = "    ".repeat(depth + 1);
  if (value === null) return "None";
  if (typeof value === "boolean") return value ? "True" : "False";
  if (typeof value === "number") return String(value);
  if (typeof value === "string") return JSON.stringify(value);
  if (Array.isArray(value)) {
    if (value.length === 0) return "[]";
    return `[\n${value.map((item) => `${childIndent}${pythonLiteral(item, depth + 1)}`).join(",\n")}\n${indent}]`;
  }
  const entries = Object.entries(value as Record<string, unknown>);
  return `{\n${entries.map(([key, item]) => `${childIndent}${JSON.stringify(key)}: ${pythonLiteral(item, depth + 1)}`).join(",\n")}\n${indent}}`;
}

export function generateApiSample(
  language: ApiSampleLanguage,
  family: ApiFamily,
  baseUrl: string,
): string {
  const definition = apiFamilyDefinition(family);
  const url = `${normalizedBaseUrl(baseUrl)}${definition.path}`;
  const jsonBody = JSON.stringify(definition.body);

  if (language === "curl") {
    return [
      `curl "${url}" \\`,
      `  -H "Authorization: Bearer $LYNSHEN_API_KEY" \\`,
      `  -H "Content-Type: application/json" \\`,
      `  -d '${jsonBody}'`,
    ].join("\n");
  }
  if (language === "python") {
    return `import os\nimport requests\n\nresponse = requests.post(\n    "${url}",\n    headers={"Authorization": f"Bearer {os.environ['LYNSHEN_API_KEY']}"},\n    json=${pythonLiteral(definition.body, 1)},\n)\nresponse.raise_for_status()\nprint(response.json())`;
  }
  if (language === "javascript") {
    return `const response = await fetch("${url}", {\n  method: "POST",\n  headers: {\n    "Authorization": \`Bearer \${process.env.LYNSHEN_API_KEY}\`,\n    "Content-Type": "application/json",\n  },\n  body: JSON.stringify(${JSON.stringify(definition.body, null, 2)}),\n});\nif (!response.ok) throw new Error(await response.text());\nconsole.log(await response.json());`;
  }
  return `package main\n\nimport (\n  "bytes"\n  "net/http"\n  "os"\n)\n\nfunc main() {\n  body := []byte(\`${jsonBody}\`)\n  req, _ := http.NewRequest("POST", "${url}", bytes.NewReader(body))\n  req.Header.Set("Authorization", "Bearer "+os.Getenv("LYNSHEN_API_KEY"))\n  req.Header.Set("Content-Type", "application/json")\n  response, err := http.DefaultClient.Do(req)\n  if err != nil { panic(err) }\n  defer response.Body.Close()\n}`;
}
