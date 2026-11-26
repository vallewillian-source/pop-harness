
import { SofaBanner, SofaButton, SofaModal } from "@sofacoding/sofa-ui";
import type { FileOrigin } from "../api/testFile";
import { useI18n } from "../i18n";
import "../styles/chat-test.css";

export type PreviewImage = {
    origem: FileOrigin;
    estado: "carregando" | "pronta" | "erro";
    src: string | null;
    erro: string | null;
};

type Props = {
    preview: PreviewImage | null;
    onClose: () => void;
    onTryNew: () => void;
};

export function PreviewFileTest({ preview, onClose, onTryNew }: Props) {
    const { translate: t } = useI18n();
    return (
        <SofaModal
            open={preview !== null}
            onOpenChange={(next) => {
                if (!next) onClose();
            }}
            title={
                preview
                    ? t("testChat.previewTitle", { name: preview.origem.nome })
                    : t("testChat.previewTitlePlain")
            }
            description={
                preview?.origem.de === "agente"
                    ? t("testChat.previewFromAgent")
                    : t("testChat.previewFromClient")
            }
            closeLabel={t("testChat.previewClose")}
            size="lg"
        >
            {preview?.estado === "carregando" && (
                <p className="poph-preview-file__state" role="status">
                    {t("testChat.previewLoading")}
                </p>
            )}
            {preview?.estado === "erro" && (
                <SofaBanner tone="danger" title={t("testChat.previewErrorTitle")}>
                    <p>{preview.erro}</p>
                    <SofaButton variant="secondary" onClick={onTryNew}>
                        {t("testChat.retry")}
                    </SofaButton>
                </SofaBanner>
            )}
            {preview?.estado === "pronta" && preview.src && (
                <img
                    className="poph-preview-file__image"
                    src={preview.src}
                    alt={
                        preview.origem.de === "agente"
                            ? t("testChat.imageFromAgent", { name: preview.origem.nome })
                            : t("testChat.imageFromClient", { name: preview.origem.nome })
                    }
                />
            )}
        </SofaModal>
    );
}
