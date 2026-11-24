# Exemplo 3 — efeitos: mensagem ao cliente, notificação para a equipe e
# encaminhamento para outro departamento. O primeiro fim encerra o atendimento;
# o segundo só conclui o fluxo e deixa a conversa com o departamento de destino.
fluxo(
    id = "confirmar_entrega",
    nome = "Confirmar entrega",
    descricao = "Confirma a entrega de um pedido e encerra o atendimento.",
    inicio = "identificacao",
    gatilho = gatilho(
        quando = "O cliente pergunta onde está o pedido ou quer confirmar a entrega.",
        nao_quando = "O cliente quer trocar, devolver ou comprar outro produto.",
        exemplos = ["meu pedido chegou?", "quando chega a entrega do pedido 1234?"],
    ),
)

etapa(
    id = "identificacao",
    nome = "Identificar o pedido",
    instrucao = "Peça o número do pedido e confirme que é esse o atendimento.",
    campos = [
        campo("pedido", "Número do pedido", tipo = "texto", obrigatorio = True),
    ],
    acoes = [
        acao("localizado", "Pedido localizado", exige = ["pedido"], vai_para = "confirmacao"),
    ],
)

etapa(
    id = "confirmacao",
    nome = "Confirmar a entrega",
    instrucao = "Pergunte se a entrega foi recebida. Não prometa prazo novo.",
    campos = [
        campo("recebido", "Entrega recebida", tipo = "opcao", opcoes = ["sim", "nao"], obrigatorio = True),
    ],
    ao_entrar = [
        mensagem_ao_cliente("Vou confirmar a entrega do pedido {identificacao.pedido} com você."),
    ],
    acoes = [
        acao(
            "confirmada",
            "Entrega confirmada",
            exige = ["recebido"],
            vai_para = fim("concluido", resultado = "Entrega confirmada", encerra_atendimento = True),
            efeitos = [
                notificar_whatsapp(
                    para = "5521999990000",
                    mensagem = "Pedido {identificacao.pedido} entregue (ficha {ficha.id}).",
                ),
            ],
        ),
        acao("nao_recebida", "Cliente não recebeu", exige = ["recebido"], vai_para = "extravio"),
    ],
)

etapa(
    id = "extravio",
    nome = "Registrar extravio",
    instrucao = "Anote o que o cliente disse e avise que a logística vai investigar.",
    acoes = [
        acao(
            "registrado",
            "Extravio registrado",
            vai_para = fim("extravio_registrado", resultado = "Extravio registrado"),
            efeitos = [encaminhar("Logística")],
        ),
    ],
)
