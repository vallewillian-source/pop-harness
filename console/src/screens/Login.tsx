
import { useState, type FormEvent } from "react";
import { useNavigate } from "react-router-dom";
import {
    IconEye,
    IconEyeOff,
    IconKey,
    SofaBanner,
    SofaButton,
    SofaField,
    SofaIconButton,
    SofaInput,
    SofaStatusIndicator,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import { LanguageSwitcher } from "../components/LanguageSwitcher";
import { useI18n } from "../i18n";
import { resolveDefaultOrgPath, useSession } from "../session/Session";
import "../styles/login.css";

function instancia(): string {
    return window.location.origin;
}

export function Login() {
    const { translate } = useI18n();
    const { signIn, expired, clearExpiryNotice } = useSession();
    const navigate = useNavigate();
    const [token, setToken] = useState("");
    const [visible, setVisible] = useState(false);
    const [errorToken, setTokenError] = useState<string | null>(null);
    const [networkError, setNetworkError] = useState<string | null>(null);
    const [sending, setSending] = useState(false);

    async function send(evento: FormEvent) {
        evento.preventDefault();
        setTokenError(null);
        setNetworkError(null);
        setSending(true);
        try {
            const session = await signIn(token);
            clearExpiryNotice();
            navigate(resolveDefaultOrgPath(session.orgs), { replace: true });
        } catch (error) {
            if (error instanceof ApiError) {
                if (error.unauthorized) {
                    setTokenError(translate("login.invalidToken"));
                } else if (error.noServer) {
                    setNetworkError(
                        translate("login.unreachableInstance", { origin: instancia() }),
                    );
                } else {
                    setTokenError(error.message);
                }
            } else {
                setNetworkError(translate("login.unexpectedError"));
            }
        } finally {
            setSending(false);
        }
    }

    return (
        <main className="poph-login" role="main">
            <section className="poph-login__block" aria-labelledby="poph-login-titulo">
                <div className="poph-login__top">
                    <div className="poph-login__mark">
                        <span className="poph-login__monogram" aria-hidden="true">
                            P
                        </span>
                        <span className="poph-login__name">POP HARNESS</span>
                    </div>
                    <LanguageSwitcher />
                </div>
                <h1 id="poph-login-titulo" className="poph-login__title">
                    {translate("login.title")}
                </h1>
                <p className="poph-login__description">{translate("login.subtitle")}</p>

                {(expired || networkError) && (
                    <div className="poph-login__notices">
                        {expired && (
                            <SofaBanner tone="warn" title={translate("login.expiredTitle")}>
                                {translate("login.expiredBody")}
                            </SofaBanner>
                        )}
                        {networkError && (
                            <SofaBanner tone="danger" title={translate("login.offlineTitle")}>
                                {networkError}
                            </SofaBanner>
                        )}
                    </div>
                )}

                <form className="poph-login__form" onSubmit={send} noValidate>
                    <SofaField
                        label={translate("login.tokenLabel")}
                        hint={translate("login.tokenHint")}
                        error={errorToken ?? undefined}
                        required
                        htmlFor="poph-token"
                    >
                        {(control) => (
                            <div className="poph-login__field">
                                <SofaInput
                                    {...control}
                                    id="poph-token"
                                    type={visible ? "text" : "password"}
                                    value={token}
                                    autoComplete="off"
                                    autoFocus
                                    spellCheck={false}
                                    placeholder={translate("login.tokenPlaceholder")}
                                    onChange={(evento) => setToken(evento.target.value)}
                                />
                                <SofaIconButton
                                    label={
                                        visible
                                            ? translate("login.hideToken")
                                            : translate("login.showToken")
                                    }
                                    size={36}
                                    type="button"
                                    onClick={() => setVisible((current) => !current)}
                                >
                                    {visible ? (
                                        <IconEyeOff aria-hidden="true" />
                                    ) : (
                                        <IconEye aria-hidden="true" />
                                    )}
                                </SofaIconButton>
                            </div>
                        )}
                    </SofaField>

                    <div className="poph-fact">
                        <span className="poph-fact__label">
                            {translate("login.instanceLabel")}
                        </span>
                        <span className="poph-fact__value poph-fact__value--mono">
                            {instancia()}
                        </span>
                    </div>

                    <SofaButton
                        type="submit"
                        variant="primary"
                        fullWidth
                        loading={sending}
                        icon={<IconKey aria-hidden="true" />}
                    >
                        {translate("login.submit")}
                    </SofaButton>

                    <p className="poph-login__notice">{translate("login.sessionNotice")}</p>

                    <SofaStatusIndicator tone="muted">
                        {translate("login.scopeNotice")}
                    </SofaStatusIndicator>
                </form>
            </section>
        </main>
    );
}
