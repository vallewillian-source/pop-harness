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
import { ErroApi } from "../api/cliente";
import { useSessao } from "../sessao/Sessao";
import "../estilos/login.css";

/** Endereço desta instância — o Console é servido pelo próprio binário, então é a origem atual. */
function instancia(): string {
    return window.location.origin;
}

export function Login() {
    const { entrar, expirada, limparAvisoExpiracao } = useSessao();
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
            // Token de org não tem o que escolher: entra direto no painel da própria org.
            const propria = sessao.escopo === "org" ? sessao.orgs[0]?.id : undefined;
            navegar(propria ? `/orgs/${propria}` : "/orgs", { replace: true });
        } catch (erro) {
            if (erro instanceof ErroApi) {
                if (erro.naoAutorizado) {
                    setErroToken(
                        "Token administrativo inválido ou revogado. Confira com a equipe de operação ou gere outro com `poph token`.",
                    );
                } else if (erro.semServidor) {
                    setErroRede(
                        `Não foi possível alcançar a instância em ${instancia()}. Verifique se o processo \`poph serve\` está em execução.`,
                    );
                } else {
                    setErroToken(erro.message);
                }
            } else {
                setErroRede("Falha inesperada ao entrar. Tente de novo.");
            }
        } finally {
            setEnviando(false);
        }
    }

    return (
        <main className="poph-login" role="main">
            <section className="poph-login__bloco" aria-labelledby="poph-login-titulo">
                <div className="poph-login__marca">
                    <span className="poph-login__monograma" aria-hidden="true">
                        P
                    </span>
                    <span className="poph-login__nome">POP HARNESS</span>
                </div>
                <h1 id="poph-login-titulo" className="poph-login__titulo">
                    Console do POP Harness
                </h1>
                <p className="poph-login__descricao">
                    Acesso administrativo por token. Uso interno.
                </p>

                {(expirada || erroRede) && (
                    <div className="poph-login__avisos">
                        {expirada && (
                            <SofaBanner tone="warn" title="Sessão encerrada">
                                O token desta sessão não é mais aceito pela instância.
                            </SofaBanner>
                        )}
                        {erroRede && (
                            <SofaBanner tone="danger" title="Sem conexão">
                                {erroRede}
                            </SofaBanner>
                        )}
                    </div>
                )}

                <form className="poph-login__form" onSubmit={enviar} noValidate>
                    <SofaField
                        label="Token administrativo"
                        hint="Gerado com `poph token` na instância, ou definido em [admin].tokens_globais."
                        error={erroToken ?? undefined}
                        required
                        htmlFor="poph-token"
                    >
                        {(controle) => (
                            <div className="poph-login__campo">
                                <SofaInput
                                    {...controle}
                                    id="poph-token"
                                    type={visivel ? "text" : "password"}
                                    value={token}
                                    autoComplete="off"
                                    autoFocus
                                    spellCheck={false}
                                    placeholder="pophg_… ou poph_…"
                                    onChange={(evento) => setToken(evento.target.value)}
                                />
                                <SofaIconButton
                                    label={visivel ? "Ocultar token" : "Mostrar token"}
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

                    <div className="poph-fato">
                        <span className="poph-fato__rotulo">Instância</span>
                        <span className="poph-fato__valor poph-fato__valor--mono">
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
                        Entrar no Console
                    </SofaButton>

                    <p className="poph-login__aviso">
                        O token fica só na memória desta aba e no armazenamento de sessão do
                        navegador: fechar a aba encerra o acesso. Nada é gravado de forma
                        permanente.
                    </p>

                    <SofaStatusIndicator tone="muted">
                        Escopo é definido pelo token: global enxerga todas as organizações, de
                        organização enxerga só a própria.
                    </SofaStatusIndicator>
                </form>
            </section>
        </main>
    );
}
