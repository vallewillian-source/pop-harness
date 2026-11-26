/**
 * Tela de login por token (UX_SPEC §3.1). Sem cadastro de usuário: o operador cola o token gerado
 * por `poph token` na CLI ou configurado na instância.
 */

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

/** Endereço desta instância — o Console é servido pelo próprio binário, então é a origem atual. */
function instancia(): string {
    return window.location.origin;
}

export function Login() {
    const { translate } = useI18n();
    const { entrar, expirada, limparAvisoExpiracao } = useSession();
    const navegar = useNavigate();
    const [token, setToken] = useState("");
    const [visivel, setVisivel] = useState(false);
    const [erroToken, setErroToken] = useState<string | null>(null);
    const [erroRede, setErroRede] = useState<string | null>(null);
    const [enviando, setEnviando] = useState(false);

    async function enviar(evento: FormEvent) {
        evento.preventDefault();
        setErroToken(null);
        setErroRede(null);
        setEnviando(true);
        try {
            const sessao = await entrar(token);
            limparAvisoExpiracao();
            // Vale para os dois escopos: token de org só enxerga a própria, que é a primeira da lista.
            navegar(resolveDefaultOrgPath(sessao.orgs), { replace: true });
        } catch (erro) {
            if (erro instanceof ApiError) {
                if (erro.naoAutorizado) {
                    setErroToken(translate("login.invalidToken"));
                } else if (erro.semServidor) {
                    setErroRede(
                        translate("login.unreachableInstance", { origin: instancia() }),
                    );
                } else {
                    setErroToken(erro.message);
                }
            } else {
                setErroRede(translate("login.unexpectedError"));
            }
        } finally {
            setEnviando(false);
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

                {(expirada || erroRede) && (
                    <div className="poph-login__notices">
                        {expirada && (
                            <SofaBanner tone="warn" title={translate("login.expiredTitle")}>
                                {translate("login.expiredBody")}
                            </SofaBanner>
                        )}
                        {erroRede && (
                            <SofaBanner tone="danger" title={translate("login.offlineTitle")}>
                                {erroRede}
                            </SofaBanner>
                        )}
                    </div>
                )}

                <form className="poph-login__form" onSubmit={enviar} noValidate>
                    <SofaField
                        label={translate("login.tokenLabel")}
                        hint={translate("login.tokenHint")}
                        error={erroToken ?? undefined}
                        required
                        htmlFor="poph-token"
                    >
                        {(controle) => (
                            <div className="poph-login__field">
                                <SofaInput
                                    {...controle}
                                    id="poph-token"
                                    type={visivel ? "text" : "password"}
                                    value={token}
                                    autoComplete="off"
                                    autoFocus
                                    spellCheck={false}
                                    placeholder={translate("login.tokenPlaceholder")}
                                    onChange={(evento) => setToken(evento.target.value)}
                                />
                                <SofaIconButton
                                    label={
                                        visivel
                                            ? translate("login.hideToken")
                                            : translate("login.showToken")
                                    }
                                    size={36}
                                    type="button"
                                    onClick={() => setVisivel((atual) => !atual)}
                                >
                                    {visivel ? (
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
                        loading={enviando}
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
