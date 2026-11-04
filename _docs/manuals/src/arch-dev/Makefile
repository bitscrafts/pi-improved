# Makefile para compilacao remota do livro de Aprendizagem de Maquina
# Servidor remoto: lnx

REMOTE_HOST = lnx
REMOTE_USER = mvcorrea
REMOTE_DIR = ~/podman/latex-remote/aprendizagem-maquina
LOCAL_DIR = .
BUILD_DIR = build
SRC_DIR = src

.PHONY: help setup sync compile download clean all test-local

help:
	@echo "Comandos disponiveis:"
	@echo "  make setup      - Configurar ambiente remoto (primeira vez)"
	@echo "  make sync       - Sincronizar arquivos locais para servidor remoto"
	@echo "  make compile    - Compilar PDF no servidor remoto"
	@echo "  make download   - Baixar PDF compilado do servidor"
	@echo "  make all        - sync + compile + download"
	@echo "  make clean      - Limpar arquivos temporarios no servidor"
	@echo "  make test-local - Testar compilacao local (requer LaTeX instalado)"

# Configurar ambiente no servidor remoto (executar apenas uma vez)
setup:
	@echo "Configurando ambiente remoto em $(REMOTE_HOST)..."
	ssh $(REMOTE_USER)@$(REMOTE_HOST) "mkdir -p $(REMOTE_DIR)/{src,build}"
	@echo "Transferindo arquivos da template kaobook..."
	rsync -az kaobook-master/*.cls kaobook-master/*.sty $(REMOTE_USER)@$(REMOTE_HOST):$(REMOTE_DIR)/
	@echo "Verificando container LaTeX compartilhado..."
	@ssh $(REMOTE_USER)@$(REMOTE_HOST) "cd ~/podman/latex-remote && podman-compose ps | grep -q latex-compiler-shared || podman-compose up -d"
	@echo "Setup completo!"

# Sincronizar arquivos fonte para o servidor
sync:
	@echo "Sincronizando arquivos para $(REMOTE_HOST)..."
	rsync -avz --delete $(SRC_DIR)/ $(REMOTE_USER)@$(REMOTE_HOST):$(REMOTE_DIR)/src/
	@echo "Sincronizacao completa!"

# Compilar no servidor remoto
compile:
	@echo "Compilando PDF no servidor remoto..."
	ssh $(REMOTE_USER)@$(REMOTE_HOST) "podman exec latex-compiler-shared sh -c '\
		cd /workspace/aprendizagem-maquina/src && \
		pdflatex -interaction=nonstopmode -output-directory=../build main.tex && \
		biber --output-directory ../build main && \
		pdflatex -interaction=nonstopmode -output-directory=../build main.tex && \
		pdflatex -interaction=nonstopmode -output-directory=../build main.tex && \
		pdflatex -interaction=nonstopmode -output-directory=../build main.tex'"
	@echo "Compilacao completa!"

# Baixar PDF compilado
download:
	@echo "Baixando PDF compilado..."
	mkdir -p $(BUILD_DIR)
	scp $(REMOTE_USER)@$(REMOTE_HOST):$(REMOTE_DIR)/build/main.pdf $(BUILD_DIR)/aprendizagem-maquina.pdf
	@echo "PDF baixado para $(BUILD_DIR)/aprendizagem-maquina.pdf"

# Executar tudo: sincronizar, compilar e baixar
all: sync compile download

# Limpar arquivos temporarios no servidor
clean:
	@echo "Limpando arquivos temporarios no servidor..."
	ssh $(REMOTE_USER)@$(REMOTE_HOST) "cd $(REMOTE_DIR)/build && rm -f *.aux *.log *.out *.toc *.bbl *.bcf *.blg *.run.xml *.fdb_latexmk *.fls *.synctex.gz"
	@echo "Limpeza completa!"

# Testar compilacao local (requer LaTeX instalado)
test-local:
	@echo "Testando compilacao local..."
	@if command -v pdflatex >/dev/null 2>&1; then \
		cd src && \
		pdflatex -output-directory=../build main.tex && \
		biber --output-directory ../build main && \
		pdflatex -output-directory=../build main.tex && \
		pdflatex -output-directory=../build main.tex && \
		pdflatex -output-directory=../build main.tex; \
	else \
		echo "ERRO: pdflatex nao encontrado. Instale TeX Live ou use compilacao remota."; \
		exit 1; \
	fi