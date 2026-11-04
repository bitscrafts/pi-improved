#!/bin/bash

# Script auxiliar para compilacao do livro de Aprendizagem de Maquina
# Uso: ./compile.sh [comando]
# Comandos: setup, sync, compile, download, all, clean, status

REMOTE_HOST="lnx"
REMOTE_USER="mvcorrea"
REMOTE_DIR="~/podman/latex-remote/pii-manuals/user-admin"
LOCAL_DIR="$(pwd)"

# Cores para output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Funcao para exibir mensagens
info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

success() {
    echo -e "${GREEN}[OK]${NC} $1"
}

error() {
    echo -e "${RED}[ERRO]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[AVISO]${NC} $1"
}

# Verificar conexao com servidor
check_connection() {
    info "Verificando conexao com ${REMOTE_HOST}..."
    if ssh -o ConnectTimeout=5 "${REMOTE_USER}@${REMOTE_HOST}" "exit" 2>/dev/null; then
        success "Conectado a ${REMOTE_HOST}"
        return 0
    else
        error "Nao foi possivel conectar a ${REMOTE_HOST}"
        error "Verifique: 1) VPN esta ativa? 2) Servidor esta acessivel? 3) Credenciais SSH corretas?"
        return 1
    fi
}

# Setup inicial
cmd_setup() {
    info "Configurando ambiente remoto..."

    check_connection || exit 1

    info "Criando estrutura de diretorios..."
    ssh "${REMOTE_USER}@${REMOTE_HOST}" "mkdir -p ${REMOTE_DIR}/{src,build}"

    info "Transferindo arquivos da template kaobook..."
    rsync -az kaobook-master/*.cls kaobook-master/*.sty "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/"

    info "Verificando container LaTeX compartilhado..."
    if ! ssh "${REMOTE_USER}@${REMOTE_HOST}" "podman ps | grep -q latex-compiler-shared"; then
        warn "Container nao esta rodando. Iniciando..."
        ssh "${REMOTE_USER}@${REMOTE_HOST}" "cd ~/podman/latex-remote && podman-compose up -d"
    fi

    success "Setup concluido com sucesso!"
    info "Proximo passo: execute './compile.sh all' para sincronizar e compilar"
}

# Sincronizar arquivos
cmd_sync() {
    info "Sincronizando arquivos para ${REMOTE_HOST}..."

    check_connection || exit 1

    rsync -avz --progress --delete src/ "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/src/"

    success "Sincronizacao completa!"
}

# Compilar
cmd_compile() {
    info "Compilando PDF no servidor remoto..."

    check_connection || exit 1

    info "Executando compilacao LaTeX (pode levar alguns minutos)..."

    ssh "${REMOTE_USER}@${REMOTE_HOST}" "podman exec latex-compiler-shared sh -c '
        cd /workspace/pii-manuals/user-admin/src && \
        echo \"=== Primeira passagem (pdflatex) ===\" && \
        pdflatex -interaction=nonstopmode -output-directory=../build main.tex; \
        echo \"=== Processando indice (makeindex) ===\" && \
        makeindex ../build/main.idx; \
        echo \"=== Processando bibliografia (biber) ===\" && \
        biber --output-directory ../build main 2>&1 | grep -v \"WARN\" || true && \
        echo \"=== Segunda passagem (pdflatex) ===\" && \
        pdflatex -interaction=nonstopmode -output-directory=../build main.tex && \
        echo \"=== Terceira passagem (pdflatex) ===\" && \
        pdflatex -interaction=nonstopmode -output-directory=../build main.tex && \
        echo \"=== Quarta passagem (pdflatex) ===\" && \
        pdflatex -interaction=nonstopmode -output-directory=../build main.tex
    '"

    if [ $? -eq 0 ]; then
        success "Compilacao concluida!"
        info "Execute './compile.sh download' para baixar o PDF"
    else
        error "Erro durante compilacao!"
        error "Execute './compile.sh logs' para ver os logs de erro"
        exit 1
    fi
}

# Download do PDF
cmd_download() {
    info "Baixando PDF compilado..."

    check_connection || exit 1

    mkdir -p build

    # Nome do PDF baseado no nome da pasta do projeto
    PROJECT_NAME="pii-user-admin"

    if scp "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/build/main.pdf" "build/${PROJECT_NAME}.pdf" 2>/dev/null; then
        success "PDF baixado para: build/${PROJECT_NAME}.pdf"

        # Verificar tamanho do arquivo
        size=$(ls -lh "build/${PROJECT_NAME}.pdf" | awk '{print $5}')
        info "Tamanho do arquivo: ${size}"

        # Abrir PDF automaticamente (macOS)
        if [[ "$OSTYPE" == "darwin"* ]]; then
            info "Abrindo PDF..."
            open "build/${PROJECT_NAME}.pdf" 2>/dev/null || true
        fi
    else
        error "Falha ao baixar PDF"
        error "Verifique se a compilacao foi bem-sucedida"
        exit 1
    fi
}

# Executar tudo
cmd_all() {
    cmd_sync && cmd_compile && cmd_download
}

# Limpar arquivos temporarios
cmd_clean() {
    info "Limpando arquivos temporarios no servidor..."

    check_connection || exit 1

    ssh "${REMOTE_USER}@${REMOTE_HOST}" "cd ${REMOTE_DIR}/build && rm -f *.aux *.log *.out *.toc *.bbl *.bcf *.blg *.run.xml *.fdb_latexmk *.fls *.synctex.gz *.idx *.ilg *.ind"

    success "Limpeza concluida!"
}

# Ver logs de erro
cmd_logs() {
    info "Obtendo logs de compilacao..."

    check_connection || exit 1

    ssh "${REMOTE_USER}@${REMOTE_HOST}" "cat ${REMOTE_DIR}/build/main.log 2>/dev/null | tail -n 50"
}

# Status do container
cmd_status() {
    info "Verificando status do container..."

    check_connection || exit 1

    ssh "${REMOTE_USER}@${REMOTE_HOST}" "cd ~/podman/latex-remote && podman-compose ps"
}

# Ajuda
cmd_help() {
    cat <<EOF
${BLUE}Compilador do Livro de Aprendizagem de Maquina${NC}

${YELLOW}Uso:${NC} ./compile.sh [comando]

${YELLOW}Comandos disponiveis:${NC}
  ${GREEN}setup${NC}      - Configurar ambiente remoto (executar uma vez)
  ${GREEN}sync${NC}       - Sincronizar arquivos locais para servidor
  ${GREEN}compile${NC}    - Compilar PDF no servidor remoto
  ${GREEN}download${NC}   - Baixar PDF compilado
  ${GREEN}all${NC}        - Executar sync + compile + download
  ${GREEN}clean${NC}      - Limpar arquivos temporarios no servidor
  ${GREEN}logs${NC}       - Ver logs de compilacao (ultimas 50 linhas)
  ${GREEN}status${NC}     - Ver status do container Docker
  ${GREEN}help${NC}       - Exibir esta ajuda

${YELLOW}Exemplos:${NC}
  ./compile.sh setup          # Primeira vez
  ./compile.sh all            # Compilar tudo
  ./compile.sh sync compile   # Sincronizar e compilar

${YELLOW}Configuracao:${NC}
  Servidor: ${REMOTE_HOST}
  Usuario:  ${REMOTE_USER}
  Diretorio: ${REMOTE_DIR}
EOF
}

# Main
case "${1:-help}" in
    setup)
        cmd_setup
        ;;
    sync)
        cmd_sync
        ;;
    compile)
        cmd_compile
        ;;
    download)
        cmd_download
        ;;
    all)
        cmd_all
        ;;
    clean)
        cmd_clean
        ;;
    logs)
        cmd_logs
        ;;
    status)
        cmd_status
        ;;
    help|--help|-h)
        cmd_help
        ;;
    *)
        error "Comando desconhecido: $1"
        echo ""
        cmd_help
        exit 1
        ;;
esac