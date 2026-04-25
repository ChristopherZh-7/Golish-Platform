import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { copyToClipboard } from "@/lib/clipboard";
import { zapDownloadRootCert, zapInstallRootCert } from "@/lib/pentest/zap-api";

export function useZapProxyCert(proxyAddr: string) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const [certLoading, setCertLoading] = useState(false);
  const [certResult, setCertResult] = useState<{ ok: boolean; msg: string } | null>(null);

  const copyProxy = useCallback(async () => {
    if (await copyToClipboard(proxyAddr)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    }
  }, [proxyAddr]);

  const handleDownloadCert = useCallback(async () => {
    setCertLoading(true);
    setCertResult(null);
    try {
      const path = await zapDownloadRootCert();
      setCertResult({ ok: true, msg: path });
    } catch (e) {
      setCertResult({ ok: false, msg: String(e) });
    } finally {
      setCertLoading(false);
    }
  }, []);

  const handleInstallCert = useCallback(async () => {
    setCertLoading(true);
    setCertResult(null);
    try {
      const path = await zapInstallRootCert();
      setCertResult({ ok: true, msg: t("browser.certInstalled", `Certificate installed: ${path}`) });
    } catch (e) {
      setCertResult({ ok: false, msg: String(e) });
    } finally {
      setCertLoading(false);
    }
  }, [t]);

  return { copied, certLoading, certResult, copyProxy, handleDownloadCert, handleInstallCert };
}
