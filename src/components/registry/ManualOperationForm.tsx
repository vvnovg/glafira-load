import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { Address, OpKind, Operation } from "../../api/types";
import { ErrorBanner } from "../common/ErrorBanner";

const KINDS: OpKind[] = ["http", "kafka_produce", "kafka_consume", "grpc", "sql"];

function emptyAddress(kind: OpKind): Address {
  switch (kind) {
    case "http":
      return { type: "http", method: "GET", path: "/" };
    case "grpc":
      return { type: "grpc", service: "", method: "" };
    case "sql":
      return { type: "sql", name: "" };
    default:
      return { type: "kafka", topic: "" };
  }
}

/** Адрес без лишних пробелов по краям: ровно то, что уходит в Rust и попадает в `id`. */
export function trimAddress(a: Address): Address {
  switch (a.type) {
    case "http":
      return { type: "http", method: a.method.trim().toUpperCase(), path: a.path.trim() };
    case "kafka":
      return { type: "kafka", topic: a.topic.trim() };
    case "grpc":
      return { type: "grpc", service: a.service.trim(), method: a.method.trim() };
    case "sql":
      return { type: "sql", name: a.name.trim() };
  }
}

/**
 * Предпросмотр `id` по правилам реестра (бэкенд пересчитывает его сам):
 * `http:<service>:<METHOD> <path>`, `kafka_produce:<topic>`, `kafka_consume:<topic>`,
 * `grpc:<service>:<package.Service/Method>`, `sql:<service>:<name>`.
 */
export function previewId(service: string, kind: OpKind, address: Address): string {
  switch (address.type) {
    case "http":
      return `http:${service}:${address.method} ${address.path}`;
    case "kafka":
      return `${kind === "kafka_consume" ? "kafka_consume" : "kafka_produce"}:${address.topic}`;
    case "grpc":
      return `grpc:${service}:${address.service}/${address.method}`;
    case "sql":
      return `sql:${service}:${address.name}`;
  }
}

type Problem = { key: "empty"; field: string } | { key: "pathSlash" } | { key: "noSpaces"; field: string };

/** Проверка адреса (уже обрезанного) до отправки: пустые части, путь без `/`, пробелы внутри имён. */
export function addressProblems(a: Address, labels: { method: string; path: string; topic: string; grpcService: string; grpcMethod: string; sqlName: string }): Problem[] {
  const out: Problem[] = [];
  const part = (value: string, field: string) => {
    if (!value) out.push({ key: "empty", field });
    else if (/\s/.test(value)) out.push({ key: "noSpaces", field });
  };
  switch (a.type) {
    case "http":
      part(a.method, labels.method);
      if (!a.path) out.push({ key: "empty", field: labels.path });
      else if (!a.path.startsWith("/")) out.push({ key: "pathSlash" });
      break;
    case "kafka":
      part(a.topic, labels.topic);
      break;
    case "grpc":
      part(a.service, labels.grpcService);
      part(a.method, labels.grpcMethod);
      break;
    case "sql":
      part(a.name, labels.sqlName);
      break;
  }
  return out;
}

/** Ручная операция: сервис, вид, адрес по виду, имя и описание. `id` показываем заранее, окончательно его считает Rust. */
export function ManualOperationForm({ path, services, onDone, onCancel }: { path: string; services: string[]; onDone: (op: Operation) => void; onCancel: () => void }) {
  const { t } = useTranslation();
  const [service, setService] = useState(services[0] ?? "");
  const [kind, setKind] = useState<OpKind>("sql");
  const [address, setAddress] = useState<Address>(emptyAddress("sql"));
  const [name, setName] = useState("");
  const [summary, setSummary] = useState("");
  const clean = trimAddress(address);
  const problems = addressProblems(clean, {
    method: t("registry.method"),
    path: t("registry.path"),
    topic: t("registry.topic"),
    grpcService: t("registry.grpcService"),
    grpcMethod: t("registry.grpcMethod"),
    sqlName: t("registry.sqlName"),
  });
  const add = useMutation({
    mutationFn: () => api.registryAddManual(path, { id: "", service, kind, name: name.trim(), summary: summary.trim(), address: clean, source: { kind: "manual", path: "" } }),
    onSuccess: onDone,
  });
  const changeKind = (k: OpKind) => {
    setKind(k);
    setAddress(emptyAddress(k));
  };
  const field = (label: string, value: string, set: (v: string) => void) => (
    <label>
      {label}
      <input aria-label={label} value={value} onChange={(e) => set(e.target.value)} />
    </label>
  );
  const problemText = (p: Problem) => (p.key === "pathSlash" ? t("registry.invalid.pathSlash") : t(`registry.invalid.${p.key}`, { field: p.field }));
  return (
    <form
      className="form panel registry-form"
      onSubmit={(e) => {
        e.preventDefault();
        if (problems.length === 0) add.mutate();
      }}
    >
      <h3>{t("registry.addManual")}</h3>
      <div className="row">
        <label>
          {t("registry.service")}
          <select aria-label={t("registry.service")} value={service} onChange={(e) => setService(e.target.value)}>
            {services.map((s) => (
              <option key={s}>{s}</option>
            ))}
          </select>
        </label>
        <label>
          {t("registry.kind")}
          <select aria-label={t("registry.kind")} value={kind} onChange={(e) => changeKind(e.target.value as OpKind)}>
            {KINDS.map((k) => (
              <option key={k} value={k}>
                {t(`registry.kinds.${k}`)}
              </option>
            ))}
          </select>
        </label>
      </div>
      <div className="row">
        {address.type === "http" && field(t("registry.method"), address.method, (v) => setAddress({ ...address, method: v.toUpperCase() }))}
        {address.type === "http" && field(t("registry.path"), address.path, (v) => setAddress({ ...address, path: v }))}
        {address.type === "kafka" && field(t("registry.topic"), address.topic, (v) => setAddress({ ...address, topic: v }))}
        {address.type === "grpc" && field(t("registry.grpcService"), address.service, (v) => setAddress({ ...address, service: v }))}
        {address.type === "grpc" && field(t("registry.grpcMethod"), address.method, (v) => setAddress({ ...address, method: v }))}
        {address.type === "sql" && field(t("registry.sqlName"), address.name, (v) => setAddress({ ...address, name: v }))}
      </div>
      <div className="row">
        {field(t("registry.name"), name, setName)}
        {field(t("registry.summary"), summary, setSummary)}
      </div>
      <p>
        {t("registry.idPreview")} <code>{previewId(service, kind, clean)}</code>
      </p>
      {problems.length > 0 && (
        <ul className="warn" role="status">
          {problems.map((p, i) => (
            <li key={i}>{problemText(p)}</li>
          ))}
        </ul>
      )}
      {add.error && <ErrorBanner error={add.error} />}
      <div className="row">
        <button type="submit" disabled={add.isPending || !service || problems.length > 0}>
          {t("common.save")}
        </button>
        <button type="button" onClick={onCancel}>
          {t("common.cancel")}
        </button>
      </div>
    </form>
  );
}
