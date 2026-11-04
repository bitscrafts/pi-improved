# Aprendizagem de Maquina - CK0193
## Table of Contents (Detailed)

### Front Matter
- Capa / Title Page
- Prefacio / Preface
- Sumario / Table of Contents
- Lista de Figuras / List of Figures
- Lista de Tabelas / List of Tables

---

## PARTE I: FUNDAMENTOS

### Capitulo 1: Introducao a Aprendizagem de Maquina

**Objetivo:** Apresentar o campo de ML, sua taxonomia e o pipeline basico, conectando com a intuicao do dia-a-dia.

#### 1.1 O que e Aprendizagem de Maquina?
- 1.1.1 Definicao formal
  - Aprender a partir de dados
  - Melhorar com experiencia
  - Aprendizagem como otimizacao
- 1.1.2 Por que aprender?
  - Tarefas dificeis de programar explicitamente
  - Dados abundantes, regras escassas
  - Adaptacao e generalizacao

**Sidebar formulas:**
- Tom Mitchell: "Programa melhora com experiencia E na tarefa T medido por P"
- Paradigma: dados -> modelo -> predicao

**Exercicios:**
- Exercicio 1.1: Identificar tipo de aprendizagem em cenarios
- Exercicio 1.2: Diferenciar regras manuais de aprendizado

#### 1.2 Tipos de Aprendizagem
- 1.2.1 Supervisionada
  - Classificacao (y discreto)
  - Regressao (y continuo)
- 1.2.2 Nao-supervisionada
  - Clustering
  - Reducao de dimensionalidade
- 1.2.3 Por reforco
  - Agente, ambiente, recompensa
  - Exploracao vs. exploracao

**Sidebar formulas:**
- Tabela: Supervisionada vs. Nao-supervisionada vs. Reforco
- Exemplos do dia-a-dia para cada tipo

**Exercicios:**
- Exercicio 1.3: Classificar problemas por tipo de aprendizagem

#### 1.3 Pipeline de Machine Learning
- 1.3.1 Coleta e preparacao de dados
- 1.3.2 Escolha do modelo
- 1.3.3 Treinamento e otimizacao
- 1.3.4 Avaliacao e validacao
- 1.3.5 Deploy e monitoramento

**Sidebar:**
- Pipeline diagram (coleta -> preprocessamento -> modelo -> avaliacao -> deploy)
- Feedback loop: avaliacao -> dados -> retreinamento

**Exercicios:**
- Exercicio 1.4: Desenhar pipeline para um problema real

#### 1.4 Overfitting e Underfitting
- 1.4.1 Viés e variância
- 1.4.2 Trade-off viés-variância
- 1.4.3 Regularizacao como solucao (preview)

**Sidebar formulas:**
- Bias^2 + Variance + Noise = Error
- Underfitting: alto viés, baixa variância
- Overfitting: baixo viés, alta variância

**Exercicios:**
- Exercicio 1.5: Identificar overfitting e underfitting em graficos

---

### Capitulo 2: Fundamentos Matematicos

**Objetivo:** Revisar a matematica necessaria, conectando cada conceito com sua aplicacao em ML.

#### 2.1 Algebra Linear
- 2.1.1 Vetores e matrizes
  - Notacao e operacoes basicas
  - Produto interno e normas
- 2.1.2 Multiplicacao de matrizes
  - Regra de multiplicacao
  - Propriedades (associatividade, distributividade)
- 2.1.3 Matrizes especiais
  - Identidade, transposta, inversa
  - Matrizes simetricas e ortogonais
- 2.1.4 Autovalores e autovetores
  - Decomposicao espectral
  - Aplicacao: PCA (preview)

**Sidebar formulas:**
- Normas: L1, L2, Lp
- Produto interno: <x, y> = x^T y
- Decomposicao: Av = λv
- Inversa: AA^{-1} = I

**Exercicios:**
- Exercicio 2.1: Operacoes com vetores e matrizes
- Exercicio 2.2: Calcular autovalores de uma matriz 2x2

#### 2.2 Calculo e Otimizacao
- 2.2.1 Derivadas e gradientes
  - Derivada parcial
  - Gradiente como direcao de maior crescimento
- 2.2.2 Regra da cadeia
  - Composicao de funcoes
  - Aplicacao: backpropagation (preview)
- 2.2.3 Otimizacao
  - Minimos locais e globais
  - Condicoes de primeira e segunda ordem
  - Funcoes convexas

**Sidebar formulas:**
- Gradiente: nabla f(x)
- Regra da cadeia: d/dx f(g(x)) = f'(g(x)) g'(x)
- Ponto critico: nabla f = 0

**Exercicios:**
- Exercicio 2.3: Calcular gradientes de funcoes
- Exercicio 2.4: Encontrar minimos de funcoes convexas

#### 2.3 Probabilidade e Estatistica
- 2.3.1 Variaveis aleatorias
  - Discretas e continuas
  - PMF, PDF, CDF
- 2.3.2 Distribuicao Normal (Gaussiana)
  - PDF e parametros
  - Propriedades
  - Teorema Central do Limite
- 2.3.3 Esperanca e variancia
  - Definicoes e propriedades
  - Covariancia e correlacao
- 2.3.4 Probabilidade condicional e Bayes
  - Teorema de Bayes
  - Prior, likelihood, posterior
  - Aplicacao: classificadores Bayesianos (preview)

**Sidebar formulas:**
- Normal: N(mu, sigma^2)
- P(y|x) = P(x|y)P(y)/P(x)
- E[X] = sum x*P(x), Var(X) = E[(X-mu)^2]

**Exercicios:**
- Exercicio 2.5: Calcular probabilidades com Bayes
- Exercicio 2.6: Esperanca e variancia de distribuicoes

#### 2.4 NumPy para Machine Learning
- 2.4.1 Arrays e operacoes vetorizadas
- 2.4.2 Broadcasting
- 2.4.3 Funcoes lambda e operacoes matriciais
- 2.4.4 Exercicios praticos

**Sidebar formulas:**
- Z-score: Z = (X - mu) / sigma
- Normalizacao MinMax: X' = (X - Xmin) / (Xmax - Xmin)

**Exercicios:**
- Exercicio 2.7: Normalizacao Z-score com NumPy
- Exercicio 2.8: Operacoes matriciais para regressao

---

### Capitulo 3: Regressao Linear

**Objetivo:** Introduzir o problema central de ML -- aprender parametros a partir de dados -- atraves do modelo mais simples possivel.

#### 3.1 O Problema de Regressao
- 3.1.1 Modelo linear
  - y = w^T x + epsilon
  - Intercepto e coeficientes
- 3.1.2 Notacao matricial
  - y = Xw + epsilon
  - Matriz de projeto
- 3.1.3 Suposicoes do modelo
  - Erros i.i.d. com distribuicao Normal
  - Homocedasticidade

**Sidebar formulas:**
- Modelo: y = w0 + w1*x1 + ... + wd*xd
- Matricial: y = Xw + epsilon
- Suposicoes: epsilon ~ N(0, sigma^2)

**Exercicios:**
- Exercicio 3.1: Interpretar coeficientes de regressao
- Exercicio 3.2: Montar matriz de projeto

#### 3.2 Funcao de Perda
- 3.2.1 Erro quadratico medio (MSE)
  - Definicao e intuicao
  - Por que quadratico e nao absoluto?
- 3.2.2 Risco esperado vs. risco empirico
  - Funcao de perda verdadeira (incognita)
  - Aproximacao pelos dados

**Sidebar formulas:**
- MSE = (1/N) * sum(yi - y^i)^2
- Risco empirico: L(w) = (1/N) ||y - Xw||^2
- RMSE = sqrt(MSE)

**Exercicios:**
- Exercicio 3.3: Calcular MSE manualmente
- Exercicio 3.4: Comparar funcoes de perda

#### 3.3 Solucao Analitica: OLS
- 3.3.1 Minimos Quadrados Ordinarios
  - Derivacao: minimizar ||y - Xw||^2
  - Condicao de primeira ordem
- 3.3.2 Solucao fechada
  - w* = (X^T X)^{-1} X^T y
  - Quando X^T X e inversivel
- 3.3.3 Propriedades do estimador OLS
  - BLUE (Gauss-Markov)
  - Insesgamento e eficiencia

**Sidebar formulas:**
- Derivacao passo a passo na margem
- w* = (X^T X)^{-1} X^T y
- Pseudoinversa: w* = X^+ y

**Exercicios:**
- Exercicio 3.5: Derivar a solucao OLS
- Exercicio 3.6: Calcular OLS para dataset artificial1d

#### 3.4 Gradient Descent
- 3.4.1 Intuicao geometrica
  - Descer a colina
  - Taxa de aprendizado
- 3.4.2 Regra de atualizacao
  - w(t) = w(t-1) + alpha * (1/N) * X^T(y - Xw)
  - Batch vs. estocastico
- 3.4.3 Convergencia
  - Condicoes de convergencia
  - Escolha do learning rate
  - Escalonamento de features

**Sidebar formulas:**
- Gradiente: nabla_w L = -(2/N) X^T(y - Xw)
- Atualizacao: w <- w - alpha * nabla_w L
- Learning rate: alpha tipico em [0.001, 0.1]

**Exercicios:**
- Exercicio 3.7: Implementar GD para regressao linear
- Exercicio 3.8: Experimentar com diferentes learning rates

#### 3.5 Stochastic Gradient Descent (SGD)
- 3.5.1 Atualizacao por amostra
  - w(t) = w(t-1) + alpha * ei * xi
  - Variacao e convergencia
- 3.5.2 Mini-batch Gradient Descent
  - Compromisso entre batch e estocastico
- 3.5.3 Variantes de otimizadores (preview)
  - Momentum, AdaGrad, RMSprop, Adam

**Sidebar formulas:**
- SGD: w <- w + alpha * (yi - w^T xi) * xi
- Mini-batch: w <- w + alpha * (1/|B|) * X_B^T(y_B - X_B w)

**Exercicios:**
- Exercicio 3.9: Implementar SGD
- Exercicio 3.10: Comparar GD e SGD (curvas de aprendizado)

#### 3.6 Normalizacao de Dados
- 3.6.1 Z-score (StandardScaler)
  - Z = (X - mu) / sigma
  - Quando usar
- 3.6.2 MinMax Scaling
  - X' = (X - Xmin) / (Xmax - Xmin)
  - Quando usar
- 3.6.3 Data leakage
  - Calcular parametros apenas no treino
  - Aplicar mesmos parametros no teste

**Sidebar formulas:**
- Z-score: Z = (X - mu) / sigma
- MinMax: X' = (X - min) / (max - min)
- IMPORTANTE: mu e sigma do TREINO apenas

**Exercicios:**
- Exercicio 3.11: Normalizar dados sem data leakage
- Exercicio 3.12: Efeito da normalizacao na convergencia

---

### Capitulo 4: Regressao Polinomial e Regularizacao

**Objetivo:** Estender a regressao linear para modelos nao-lineares e controlar o overfitting com regularizacao. Conecta com o trade-off viés-variância do Cap. 1.

#### 4.1 Regressao Polinomial
- 4.1.1 Features polinomiais
  - phi(x) = [1, x, x^2, ..., x^M]
  - Matriz de projeto Phi
- 4.1.2 Modelo linear nos parametros
  - Ainda e regressao linear!
  - w*(Phi) = (Phi^T Phi)^{-1} Phi^T y
- 4.1.3 Escolha do grau
  - Grau baixo: underfitting
  - Grau alto: overfitting

**Sidebar formulas:**
- Phi = [1, x, x^2, ..., x^M]
- Ainda OLS: w* = (Phi^T Phi)^{-1} Phi^T y
- Grau M: M+1 parametros

**Exercicios:**
- Exercicio 4.1: Criar features polinomiais
- Exercicio 4.2: Ajustar polinomios de graus 1 a 13

#### 4.2 Overfitting na Regressao Polinomial
- 4.2.1 Risco de overfitting
  - Modelo memoriza ruido
  - RMSE treino vs. teste
- 4.2.2 Diagnostico visual
  - Curvas de RMSE por grau
  - Diferenca treino/teste
- 4.2.3 Analise dos pesos
  - Pesos grandes = overfitting
  - Instabilidade numerica

**Sidebar formulas:**
- RMSE_treino diminui com grau
- RMSE_teste tem forma de U
- Pesos grandes: ||w||^2 >> 0

**Exercicios:**
- Exercicio 4.3: Plotar RMSE treino vs teste por grau
- Exercicio 4.4: Identificar overfitting nos pesos

#### 4.3 Regularizacao L2 (Ridge)
- 4.3.1 Motivacao: penalizar pesos grandes
  - Funcao de perda regularizada
  - L(w) = ||y - Xw||^2 + lambda||w||^2
- 4.3.2 Solucao fechada
  - w* = (lambda*I + X^T X)^{-1} X^T y
  - Efeito do lambda
- 4.3.3 Shrinkage
  - Pesos encolhem para zero
  - Estabilidade numerica (X^T X + lambda*I sempre inversivel)

**Sidebar formulas:**
- Ridge: L(w) = ||y - Xw||^2 + lambda||w||^2
- w*_ridge = (lambda*I + X^T X)^{-1} X^T y
- lambda -> 0: OLS
- lambda -> inf: w -> 0

**Exercicios:**
- Exercicio 4.5: Implementar Ridge Regression
- Exercicio 4.6: Comparar OLS vs Ridge em graus altos

#### 4.4 Regularizacao L1 (Lasso)
- 4.4.1 Penalidade L1
  - L(w) = ||y - Xw||^2 + lambda sum|w_i|
  - Sparsity (esparsidade)
- 4.4.2 Selecao de features automatica
  - Pesos zeram exatamente
  - Interpretacao do modelo
- 4.4.3 Elastic Net
  - Combinacao L1 + L2
  - Melhor de dois mundos

**Sidebar formulas:**
- Lasso: L(w) = ||y - Xw||^2 + lambda||w||_1
- Elastic Net: L(w) = ||y - Xw||^2 + lambda1||w||_1 + lambda2||w||^2
- L1: sparsity, L2: shrinkage

**Exercicios:**
- Exercicio 4.7: Comparar L1 e L2 geometricamente
- Exercicio 4.8: Efeito do lambda nos pesos

#### 4.5 Viés-Variância Trade-off
- 4.5.1 Decomposicao do erro
  - Erro = Viés^2 + Variância + Ruido
  - Visualizacao intuitiva
- 4.5.2 Efeito da regularizacao
  - Lambda alto: alto viés, baixa variância (underfitting)
  - Lambda baixo: baixo viés, alta variância (overfitting)
- 4.5.3 Validacao cruzada para escolha de lambda
  - K-fold CV (preview do Cap. 10)

**Sidebar formulas:**
- E[(y - y^)^2] = Bias^2 + Variance + Noise
- Regularizacao aumenta viés, reduz variância
- CV: escolher lambda que minimiza erro de validacao

**Exercicios:**
- Exercicio 4.9: Plotar curvas de viés-variância
- Exercicio 4.10: Escolher lambda via validacao cruzada

---

## PARTE II: CLASSIFICACAO

### Capitulo 5: Regressao Logistica

**Objetivo:** Adaptar as ideias de regressao para classificacao, introduzindo funcoes de ativacao e probabilidade condicional.

#### 5.1 De Regressao para Classificacao
- 5.1.1 Por que nao usar regressao linear para classificacao?
  - Saida fora de [0,1]
  - Sensivel a outliers
- 5.1.2 Funcao sigmoide
  - sigma(z) = 1 / (1 + e^{-z})
  - Propriedades: sigma(-z) = 1 - sigma(z)
  - Mapeia para [0,1]

**Sidebar formulas:**
- Sigmoide: sigma(z) = 1/(1+e^{-z})
- Derivada: sigma'(z) = sigma(z)(1-sigma(z))
- Probabilidade: P(y=1|x) = sigma(w^T x)

**Exercicios:**
- Exercicio 5.1: Plotar e analisar a sigmoide
- Exercicio 5.2: Calcular probabilidades com sigmoide

#### 5.2 Modelo de Regressao Logistica
- 5.2.1 Formulacao probabilistica
  - P(y=1|x) = sigma(w^T x)
  - P(y=0|x) = 1 - sigma(w^T x)
- 5.2.2 Maximum Likelihood Estimation (MLE)
  - Funcao de verossimilhanca
  - Log-verossimilhanca
  - Cross-entropy

**Sidebar formulas:**
- Likelihood: L(w) = prod sigma(w^T xi)^{yi} * (1-sigma(w^T xi))^{1-yi}
- Log-likelihood: l(w) = sum[yi*log(sigma) + (1-yi)*log(1-sigma)]
- Cross-entropy: -l(w) = -sum[yi*log(sigma) + (1-yi)*log(1-sigma)]

**Exercicios:**
- Exercicio 5.3: Derivar a log-verossimilhanca
- Exercicio 5.4: Calcular cross-entropy

#### 5.3 Otimizacao via Gradient Descent
- 5.3.1 Gradiente da cross-entropy
  - nabla_w L = (1/N) X^T(sigma(Xw) - y)
  - Semelhanca com regressao linear!
- 5.3.2 Regra de atualizacao
  - w <- w - alpha * nabla_w L
  - SGD para classificacao

**Sidebar formulas:**
- Gradiente: nabla L = (1/N) X^T(h - y) onde h = sigma(Xw)
- Atualizacao: w <- w - alpha * (1/N) X^T(h-y)
- Mesma forma que regressao linear, mas com h = sigma(Xw)!

**Exercicios:**
- Exercicio 5.5: Implementar regressao logistica com GD
- Exercicio 5.6: Implementar regressao logistica com SGD

#### 5.4 Regressao Softmax (Multiclasse)
- 5.4.1 Generalizacao para K classes
  - P(y=k|x) = exp(w_k^T x) / sum_j exp(w_j^T x)
- 5.4.2 One-vs-All e One-vs-One
  - Estrategias de decomposicao
- 5.4.3 Cross-entropy multiclasse
  - Loss = -sum_k y_k * log(P(y=k|x))

**Sidebar formulas:**
- Softmax: P(y=k|x) = exp(w_k^T x) / Z
- Z = sum_j exp(w_j^T x) (particao)
- Loss = -sum_k y_k * log(P(y=k|x))

**Exercicios:**
- Exercicio 5.7: Implementar Softmax Regression
- Exercicio 5.8: Comparar One-vs-All vs Softmax

#### 5.5 Metricas de Classificacao (Introducao)
- 5.5.1 Acuracia
- 5.5.2 Matriz de confusao
- 5.5.3 Precisao, Recall, F1-Score
- (Detalhado no Cap. 10)

**Sidebar formulas:**
- Acuracia = (TP+TN)/(TP+TN+FP+FN)
- Precisao = TP/(TP+FP)
- Recall = TP/(TP+FN)
- F1 = 2*P*R/(P+R)

**Exercicios:**
- Exercicio 5.9: Calcular metricas de classificacao

---

### Capitulo 6: Classificadores Estatisticos

**Objetivo:** Apresentar classificadores baseados em probabilidade, conectando com o Teorema de Bayes do Cap. 2.

#### 6.1 Analise do Discriminante Gaussiano (GDA)
- 6.1.1 Modelo generativo
  - P(x|y) ~ Normal multivariada
  - Estimar parametros por classe
- 6.1.2 Discriminante linear (LDA)
  - Covariancia compartilhada Sigma
  - Fronteira de decisao linear
- 6.1.3 Discriminante quadratico (QDA)
  - Covariancia por classe Sigma_k
  - Fronteira quadratica

**Sidebar formulas:**
- GDA: P(x|y=k) ~ N(mu_k, Sigma_k)
- LDA: fronteira linear (Sigma compartilhada)
- QDA: fronteira quadratica (Sigma_k por classe)
- Predicao: argmax_k P(y=k|x) = argmax_k P(x|y=k)P(y=k)

**Exercicios:**
- Exercicio 6.1: Implementar GDA binario
- Exercicio 6.2: Comparar LDA vs QDA

#### 6.2 Naive Bayes Gaussiano
- 6.2.1 Suposicao de independencia condicional
  - P(x|y=k) = prod_i P(x_i|y=k)
  - Simplificacao dramatica
- 6.2.2 Estimativa de parametros
  - mu_ik e sigma_ik para cada feature e classe
  - Razoavel com poucos dados
- 6.2.3 Predicao
  - Log-probabilidades para estabilidade numerica
  - argmax_k log P(y=k) + sum_i log P(x_i|y=k)

**Sidebar formulas:**
- Naive Bayes: P(y=k|x) prop. P(y=k) * prod_i P(x_i|y=k)
- Log: argmax_k [log P(y=k) + sum_i log P(x_i|y=k)]
- Gaussiana: P(x_i|y=k) = N(x_i; mu_ik, sigma_ik^2)

**Exercicios:**
- Exercicio 6.3: Implementar Naive Bayes
- Exercicio 6.4: Comparar GDA vs Naive Bayes

#### 6.3 Classificadores Generativos vs. Discriminativos
- 6.3.1 Modelos generativos
  - Modelam P(x|y) e P(y)
  - GDA, Naive Bayes
- 6.3.2 Modelos discriminativos
  - Modelam P(y|x) diretamente
  - Regressao Logistica, SVM
- 6.3.3 Quando usar cada um
  - Poucos dados: generativos
  - Muitos dados: discriminativos

**Sidebar:**
- Generativo: P(x|y)P(y) -> P(y|x)
- Discriminativo: P(y|x) diretamente
- Teorema: assintoticamente, discriminativo melhor
- Com poucos dados: generativo converge mais rapido

**Exercicios:**
- Exercicio 6.5: Comparar modelos em dataset Breast Cancer
- Exercicio 6.6: Analisar vantagens e desvantagens

---

### Capitulo 7: K-Nearest Neighbors

**Objetivo:** Introduzir o algoritmo baseado em instancia mais simples, conectando com ideias de distancia e similaridade.

#### 7.1 Algoritmo KNN
- 7.1.1 Intuicao: "diga-me com quem andas..."
  - Classificacao por votacao majoritaria
  - Regressao por media dos vizinhos
- 7.1.2 Algoritmo
  1. Calcular distancias para todos os pontos de treino
  2. Selecionar os K mais proximos
  3. Votacao (classificacao) ou media (regressao)
- 7.1.3 Escolha de K
  - K=1: overfitting
  - K=N: underfitting
  - Regra pratica: K impar para classificacao binaria

**Sidebar formulas:**
- Distancia Euclidiana: d(x,x') = sqrt(sum(xi-xi')^2)
- Outras: Manhattan, Minkowski, Mahalanobis
- Votacao: y^ = mode(y1,...,yK)

**Exercicios:**
- Exercicio 7.1: Implementar KNN
- Exercicio 7.2: Experimentar com diferentes K

#### 7.2 Metricas de Distancia
- 7.2.1 Distancia Euclidiana (L2)
- 7.2.2 Distancia de Manhattan (L1)
- 7.2.3 Distancia de Minkowski (Lp)
- 7.2.4 Distancia de Mahalanobis

**Sidebar formulas:**
- L2: d = sqrt(sum(xi-xi')^2)
- L1: d = sum|xi-xi'|
- Lp: d = (sum|xi-xi'|^p)^{1/p}
- Mahalanobis: d = sqrt((x-mu)^T Sigma^{-1} (x-mu))

**Exercicios:**
- Exercicio 7.3: Comparar metricas de distancia

#### 7.3 Maldicao da Dimensionalidade
- 7.3.1 Espalhamento dos dados em altas dimensoes
  - Volume cresce exponencialmente
  - Dados ficam "esparsos"
- 7.3.2 KNN em alta dimensao
  - Todas as distancias tendem a ser iguais
  - Necessidade exponencial de dados
- 7.3.3 Solucoes
  - Reducao de dimensionalidade (PCA, Cap. 16)
  - Selecao de features

**Sidebar:**
- Maldicao: volume em d dimensoes cresce como c^d
- Regra pratica: N >> 10^d amostras necessarias
- Solucao: reduzir dimensao antes de aplicar KNN

**Exercicios:**
- Exercicio 7.4: Experimentar maldicao da dimensionalidade
- Exercicio 7.5: Comparar KNN com e sem PCA

---

### Capitulo 8: Arvores de Decisao

**Objetivo:** Apresentar modelos nao-parametricos que criam regras de decisao hierarquicas, conectando com ideias de particionamento do espaco.

#### 8.1 Conceito de Arvore de Decisao
- 8.1.1 Estrutura da arvore
  - Raiz, nos internos, folhas
  - Regras de decisao if-then-else
- 8.1.2 Interpretabilidade
  - Regras faceis de entender
  - Visualizacao intuitiva

**Sidebar formulas:**
- Estrutura: raiz -> no interno -> folha
- Regra: se x_j <= t entao esquerda, senao direita
- Predicao: y^ = media (regressao) ou majoritaria (classificacao)

**Exercicios:**
- Exercicio 8.1: Desenhar arvore para um dataset simples

#### 8.2 Algoritmo CART
- 8.2.1 Divisao otima
  - Busca pela melhor feature e threshold
  - Criterios de impureza
- 8.2.2 Gini Impurity (classificacao)
  - Gini(t) = 1 - sum_k p_k^2
  - Reducao de impureza
- 8.2.3 Entropia e Information Gain
  - Entropy(t) = -sum_k p_k log2(p_k)
  - Gain = Entropy(parent) - sum(n_child/n_parent)*Entropy(child)
- 8.2.4 Variance Reduction (regressao)
  - Reducao de variancia

**Sidebar formulas:**
- Gini: G = 1 - sum p_k^2 (0 = puro, 0.5 = max impureza binaria)
- Entropy: H = -sum p_k log2(p_k)
- Info Gain: IG = H(parent) - sum (n_child/n_parent) * H(child)
- Var Reduction: VR = Var(parent) - sum (n_child/n_parent) * Var(child)

**Exercicios:**
- Exercicio 8.2: Calcular Gini e Entropy para conjuntos
- Exercicio 8.3: Encontrar melhor divisao

#### 8.3 Poda (Pruning)
- 8.3.1 Pre-pruning
  - max_depth, min_samples_split, min_samples_leaf
- 8.3.2 Post-pruning (cost-complexity)
  - Parametro alpha
  - Trade-off complexidade vs erro

**Sidebar formulas:**
- Cost-complexity: R_alpha(T) = R(T) + alpha * |T|
- |T| = numero de folhas
- alpha alto: arvore menor (mais podada)

**Exercicios:**
- Exercicio 8.4: Experimentar pre-pruning
- Exercicio 8.5: Cost-complexity pruning

#### 8.4 Arvores para Regressao
- 8.4.1 Predicao como media dos valores na folha
- 8.4.2 Variancia como criterio de divisao
- 8.4.3 Limitacoes
  - Superficies de decisao em degraus
  - Nao extrapola

**Sidebar formulas:**
- Predicao: y^_folha = media(yi na folha)
- Criterio: min sum Var(y_esquerda) + sum Var(y_direita)

**Exercicios:**
- Exercicio 8.6: Implementar arvore de regressao

---

## PARTE III: AVALIACAO E OTIMIZACAO

### Capitulo 9: Metricas de Avaliacao

**Objetivo:** Saber medir o que fazemos. Conecta com todos os capitulos anteriores -- sem metricas, nao podemos comparar modelos.

#### 9.1 Metricas de Regressao
- 9.1.1 MSE, RMSE, MAE
- 9.1.2 R^2 (coeficiente de determinacao)
- 9.1.3 MAPE

**Sidebar formulas:**
- MSE = (1/N) sum(yi - y^i)^2
- RMSE = sqrt(MSE)
- R^2 = 1 - SS_res/SS_tot
- MAE = (1/N) sum|yi - y^i|

**Exercicios:**
- Exercicio 9.1: Calcular todas as metricas de regressao

#### 9.2 Metricas de Classificacao
- 9.2.1 Matriz de confusao
- 9.2.2 Acuracia, Precisao, Recall, F1
- 9.2.3 Especificidade e Sensibilidade

**Sidebar formulas:**
- Acuracia = (TP+TN)/(TP+TN+FP+FN)
- Precisao = TP/(TP+FP)
- Recall = TP/(TP+FN)
- F1 = 2*P*R/(P+R)

**Exercicios:**
- Exercicio 9.2: Calcular matriz de confusao e metricas

#### 9.3 Curvas ROC e AUC
- 9.3.1 TPR (Recall) vs FPR
- 9.3.2 Curva ROC
- 9.3.3 AUC como area sob a curva
- 9.3.4 Curva Precision-Recall

**Sidebar formulas:**
- TPR = TP/(TP+FN) (Recall)
- FPR = FP/(FP+TN)
- AUC = area sob curva ROC
- AUC = 1.0: perfeito, 0.5: aleatorio

**Exercicios:**
- Exercicio 9.3: Plotar curva ROC e calcular AUC

#### 9.4 Validacao Cruzada
- 9.4.1 Hold-out
- 9.4.2 K-Fold Cross-Validation
- 9.4.3 Stratified K-Fold
- 9.4.4 Nested K-Fold (para selecao de hiperparametros)

**Sidebar formulas:**
- K-Fold: dividir em K folds, treinar em K-1, testar em 1
- Resultado: media +- desvio padrao das K metricas
- Nested: loop externo (avaliacao) + loop interno (tuning)

**Exercicios:**
- Exercicio 9.4: Implementar 10-Fold CV
- Exercicio 9.5: Nested CV para selecao de hiperparametros

#### 9.5 Learning Curves e Validation Curves
- 9.5.1 Learning curves: erro vs tamanho do dataset
  - Diagnostico de underfitting/overfitting
  - Verificar se mais dados ajudam
- 9.5.2 Validation curves: erro vs hiperparametro
  - Escolha otima de hiperparametros
  - Efeito do grau polinomial, lambda, etc.

**Sidebar:**
- Learning curve: erro treino e validacao vs N
- Se gap grande: overfitting (precisa de mais dados ou regularizacao)
- Se erro alto em ambos: underfitting (precisa de modelo mais complexo)

**Exercicios:**
- Exercicio 9.6: Interpretar learning curves
- Exercicio 9.7: Plotar validation curves para lambda

---

## PARTE IV: MODELOS NAO-LINEARES E AVANCADOS

### Capitulo 10: Redes Neurais Artificiais

**Objetivo:** Estender a regressao logistica para modelos nao-lineares poderosos via composicao de funcoes, conectando com GD (Cap. 3), logistica (Cap. 5) e regularizacao (Cap. 4).

#### 10.1 Perceptron
- 10.1.1 Modelo do Perceptron
  - y = sign(w^T x + b)
  - Regra de atualizacao
- 10.1.2 Teorema da Convergencia
  - Dados linearmente separaveis
  - Convergencia em tempo finito

**Sidebar formulas:**
- Perceptron: y = sign(w^T x + b)
- Atualizacao: w <- w + alpha * (yi - y^i) * xi
- Convergencia garantida para dados linearmente separaveis

**Exercicios:**
- Exercicio 10.1: Implementar Perceptron
- Exercicio 10.2: Verificar convergencia em dados linearmente separaveis

#### 10.2 Multilayer Perceptron (MLP)
- 10.2.1 Arquitetura
  - Camada de entrada, camadas ocultas, camada de saida
  - Composicao de funcoes: h = sigma(W1 x + b1), y = sigma(W2 h + b2)
- 10.2.2 Teorema da Aproximacao Universal
  - Uma camada oculta com neuronios suficientes
  - Limitacao: quantos neuronios?
- 10.2.3 Funcoes de ativacao
  - Sigmoid, Tanh, ReLU, Leaky ReLU, Softmax

**Sidebar formulas:**
- Forward: h^(l) = sigma(W^(l) h^(l-1) + b^(l))
- Sigmoid: sigma(z) = 1/(1+e^{-z})
- ReLU: f(z) = max(0, z)
- Softmax: sigma(z_k) = exp(z_k)/sum exp(z_j)

**Exercicios:**
- Exercicio 10.3: Calcular forward pass manualmente
- Exercicio 10.4: Comparar funcoes de ativacao

#### 10.3 Backpropagation
- 10.3.1 Regra da cadeia
  - Conecta com Calculo (Cap. 2)
  - Gradiente por camada
- 10.3.2 Forward pass e backward pass
  - Calcular saida, calcular erro, propagar gradientes
- 10.3.3 Atualizacao de pesos
  - Gradiente da funcao de perda
  - Learning rate e convergencia

**Sidebar formulas:**
- Loss: L = (1/N) sum l(yi, y^i)
- Chain rule: dL/dW^(l) = dL/dh^(l) * dh^(l)/dW^(l)
- Atualizacao: W^(l) <- W^(l) - alpha * dL/dW^(l)

**Exercicios:**
- Exercicio 10.5: Implementar backpropagation
- Exercicio 10.6: Verificar gradientes com verificacao numerica

#### 10.4 Treinamento de Redes Neurais
- 10.4.1 Inicializacao de pesos
  - Xavier/Glorot, He
  - Problema do gradiente que desaparece/explode
- 10.4.2 Otimizadores
  - SGD, Momentum, Adam
  - Learning rate scheduling
- 10.4.3 Regularizacao
  - L1, L2 (conecta com Cap. 4)
  - Dropout
  - Early stopping
  - Batch normalization

**Sidebar formulas:**
- Xavier: W ~ N(0, 2/(n_in + n_out))
- Momentum: v <- beta*v + alpha*grad; w <- w - v
- Adam: combina momentum e adaptacao

**Exercicios:**
- Exercicio 10.7: Experimentar diferentes inicializacoes
- Exercicio 10.8: Comparar otimizadores

---

### Capitulo 11: Support Vector Machines

**Objetivo:** Apresentar SVM como otimizacao de margem maxima, conectando com algebra linear (Cap. 2) e regularizacao (Cap. 4).

#### 11.1 Margem Maxima
- 11.1.1 Classificador de margem maxima
  - Hiperplano separador
  - Margem como distancia ao hiperplano
- 11.1.2 Formulacao primal (Hard Margin)
  - min (1/2)||w||^2 s.t. yi(w^T xi + b) >= 1
  - Conecta com regularizacao L2!

**Sidebar formulas:**
- Hiperplano: w^T x + b = 0
- Margem: 2/||w||
- Hard Margin SVM: min (1/2)||w||^2 s.t. yi(w^T xi+b) >= 1

**Exercicios:**
- Exercicio 11.1: Calcular margem de um hiperplano

#### 11.2 Soft Margin SVM
- 11.2.1 Violacoes da margem (slack variables)
  - Permitir erros de classificacao
  - Parametro C: trade-off margem vs erros
- 11.2.2 Formulacao
  - min (1/2)||w||^2 + C sum xi_i
  - s.t. yi(w^T xi + b) >= 1 - xi_i, xi_i >= 0
- 11.2.3 Efeito do parametro C
  - C alto: menos erros, margem menor
  - C baixo: mais erros, margem maior

**Sidebar formulas:**
- Soft Margin: min (1/2)||w||^2 + C sum xi_i
- xi_i = max(0, 1 - yi(w^T xi+b))
- C grande: margem rigida, C pequeno: margem suave

**Exercicios:**
- Exercicio 11.2: Experimentar com diferentes valores de C

#### 11.3 Kernel Trick
- 11.3.1 Mapeamento para espaco de maior dimensao
  - phi(x): R^d -> R^D (D >> d)
- 11.3.2 Kernel como produto interno
  - K(x,x') = phi(x)^T phi(x')
  - Nao precisamos calcular phi explicitamente!
- 11.3.3 Kernels comuns
  - Linear: K(x,x') = x^T x'
  - Polinomial: K(x,x') = (gamma*x^T x' + r)^d
  - RBF (Gaussiano): K(x,x') = exp(-gamma||x-x'||^2)
  - Sigmoid: K(x,x') = tanh(gamma*x^T x' + r)

**Sidebar formulas:**
- Kernel trick: K(x,x') = phi(x)^T phi(x')
- RBF: K(x,x') = exp(-gamma||x-x'||^2)
- gamma grande: fronteira complexa (overfitting)
- gamma pequeno: fronteira suave (underfitting)

**Exercicios:**
- Exercicio 11.3: Visualizar fronteiras com diferentes kernels
- Exercicio 11.4: Comparar Linear, Polynomial e RBF

#### 11.4 SVM Multiclasse
- 11.4.1 One-vs-All
  - K classificadores binarios
- 11.4.2 One-vs-One
  - K(K-1)/2 classificadores binarios

**Sidebar formulas:**
- OvA: K classificadores, predicao = argmax_k f_k(x)
- OvO: K(K-1)/2 classificadores, predicao = votacao majoritaria

**Exercicios:**
- Exercicio 11.5: Implementar SVM multiclasse

---

### Capitulo 12: Ensemble Methods

**Objetivo:** Combinar modelos fracos para criar modelos fortes, conectando com arvores (Cap. 8), viés-variância (Cap. 4) e regularizacao.

#### 12.1 A ideia de Ensemble
- 12.1.1 Por que combinar modelos?
  - Reduzir variância (bagging)
  - Reduzir viés (boosting)
  - Sabedoria da multidao
- 12.1.2 Condicoes para ensemble funcionar
  - Diversidade entre modelos
  - Acuracia individual > aleatorio

**Sidebar formulas:**
- Ensemble: y^ = (1/M) sum_m f_m(x) (regressao)
- Votacao: y^ = mode(f_1(x), ..., f_M(x)) (classificacao)
- Condicao: diversidade + acuracia individual

**Exercicios:**
- Exercicio 12.1: Por que ensemble funciona? (simulacao)

#### 12.2 Bagging e Random Forests
- 12.2.1 Bootstrap Aggregating
  - Amostragem com reposicao
  - Treinar M modelos em M bootstrap samples
  - Agregar por media (regressao) ou votacao (classificacao)
- 12.2.2 Random Forests
  - Bagging de arvores + feature sampling aleatorio
  - Decorrelaciona as arvores
  - Parametros: n_estimators, max_features, max_depth
- 12.2.3 Out-of-Bag (OOB) Error
  - Estimativa de erro sem validacao cruzada

**Sidebar formulas:**
- Bagging: y^ = (1/M) sum_m f_m(x)
- Random Forest: em cada divisao, testar apenas sqrt(d) features
- OOB: erro nos dados nao usados no bootstrap

**Exercicios:**
- Exercicio 12.2: Implementar Bagging manualmente
- Exercicio 12.3: Comparar Random Forest vs arvore unica

#### 12.3 Boosting
- 12.3.1 A ideia do boosting
  - Modelos sequenciais corrigem erros
  - Reduz viés (underfitting)
- 12.3.2 AdaBoost
  - Pesos adaptativos para amostras
  - alpha_m = (1/2) ln((1-epsilon_m)/epsilon_m)
  - Atualizacao de pesos: D_{m+1}(i) prop D_m(i) * exp(-alpha_m * yi * hm(xi))
- 12.3.3 Gradient Boosting
  - Ajustar gradiente da funcao de perda
  - f_m(x) = f_{m-1}(x) + alpha * h_m(x)
  - XGBoost, LightGBM, CatBoost

**Sidebar formulas:**
- AdaBoost: f(x) = sum_m alpha_m * h_m(x)
- alpha_m = (1/2) ln((1-eps_m)/eps_m)
- Gradient Boosting: f_m = f_{m-1} + alpha*h_m
- h_m ajusta os residuos de f_{m-1}

**Exercicios:**
- Exercicio 12.4: Implementar AdaBoost
- Exercicio 12.5: Comparar AdaBoost vs Gradient Boosting

#### 12.4 Stacking e Voting
- 12.4.1 Voting Classifiers
  - Hard voting: votacao majoritaria
  - Soft voting: media das probabilidades
- 12.4.2 Stacking
  - Meta-learner sobre predicoes dos base learners
  - Cuidado com overfitting

**Sidebar formulas:**
- Hard voting: y^ = mode(f_1(x), ..., f_M(x))
- Soft voting: y^ = argmax_k (1/M) sum_m P_k(y=k|x)
- Stacking: y^ = g(f_1(x), f_2(x), ..., f_M(x))

**Exercicios:**
- Exercicio 12.6: Implementar Voting Classifier
- Exercicio 12.7: Comparar Bagging vs Boosting vs Stacking

---

## PARTE V: APRENDIZAGEM NAO-SUPERVISIONADA

### Capitulo 13: Clustering

**Objetivo:** Descobrir estrutura nos dados sem labels, conectando com distancia (Cap. 7) e avaliacao (Cap. 9).

#### 13.1 O Problema do Clustering
- 13.1.1 Agrupar dados similares
  - Sem labels (nao-supervisionado)
  - Descobrir padroes e estrutura
- 13.1.2 Tipos de clustering
  - Particional, hierarquico, baseado em densidade

**Sidebar:**
- Clustering: descobrir grupos sem labels
- Diferenca de classificacao: sem y definido a priori
- Aplicacoes: segmentacao, compressao, deteccao de anomalias

**Exercicios:**
- Exercicio 13.1: Identificar clusters visualmente

#### 13.2 K-Means
- 13.2.1 Algoritmo
  1. Inicializar K centroides
  2. Atribuir cada ponto ao centroide mais proximo
  3. Recalcular centroides
  4. Repetir ate convergencia
- 13.2.2 Funcao objetivo
  - WCSS = sum_k sum_{xi em C_k} ||xi - mu_k||^2
  - Algoritmo tipo EM
- 13.2.3 Inicializacao: K-Means++
  - Escolha inteligente de centroides iniciais
  - Melhor convergencia

**Sidebar formulas:**
- K-Means objetivo: min sum_k sum_{xi em C_k} ||xi - mu_k||^2
- Passo E: atribuir pontos ao centroide mais proximo
- Passo M: recalcular centroides como media do cluster
- K-Means++: inicializacao espalhada

**Exercicios:**
- Exercicio 13.2: Implementar K-Means
- Exercicio 13.3: Comparar inicializacao aleatoria vs K-Means++

#### 13.3 Escolha de K
- 13.3.1 Elbow Method
  - WCSS vs K: procurar "cotovelo"
- 13.3.2 Silhouette Score
  - s(i) = (b(i) - a(i)) / max(a(i), b(i))
  - a(i): distancia media intra-cluster
  - b(i): distancia media ao cluster mais proximo
- 13.3.3 Gap Statistic
  - Comparar com distribuicao aleatoria

**Sidebar formulas:**
- Silhouette: s(i) = (b(i)-a(i)) / max(a(i),b(i))
- s em [-1, 1], ideal: proximo de 1
- Elbow: WCSS vs K, escolher ponto de inflexao

**Exercicios:**
- Exercicio 13.4: Escolher K com Elbow e Silhouette

#### 13.4 DBSCAN
- 13.4.1 Clustering baseado em densidade
  - epsilon: raio de vizinhanca
  - MinPts: minimo de pontos para core point
- 13.4.2 Tipos de pontos
  - Core point, border point, noise
- 13.4.3 Vantagens
  - Encontra clusters de forma arbitraria
  - Detecta outliers automaticamente
  - Nao requer K pre-definido

**Sidebar formulas:**
- Core point: |N_epsilon(x)| >= MinPts
- N_epsilon(x) = {x' : d(x,x') <= epsilon}
- Border: vizinho de core, mas nao e core
- Noise: nem core, nem border

**Exercicios:**
- Exercicio 13.5: Implementar DBSCAN
- Exercicio 13.6: Comparar K-Means vs DBSCAN

#### 13.5 Clustering Hierarquico
- 13.5.1 Agglomerative (bottom-up)
  - Cada ponto e um cluster
  - Merge os mais proximos iterativamente
- 13.5.2 Linkage
  - Single: distancia minima
  - Complete: distancia maxima
  - Average: distancia media
  - Ward: minimizar variancia
- 13.5.3 Dendrograma
  - Visualizacao da hierarquia
  - Cortar em nivel desejado

**Sidebar formulas:**
- Single linkage: d(A,B) = min d(ai,bj)
- Complete linkage: d(A,B) = max d(ai,bj)
- Average linkage: d(A,B) = (1/|A||B|) sum d(ai,bj)
- Ward: minimizar aumento de WCSS

**Exercicios:**
- Exercicio 13.7: Construir dendrograma
- Exercicio 13.8: Comparar metodos de linkage

#### 13.6 Metricas de Avaliacao de Clustering
- 13.6.1 Silhouette Score (detalhado)
- 13.6.2 Davies-Bouldin Index (menor = melhor)
- 13.6.3 Calinski-Harabasz Index (maior = melhor)

**Sidebar formulas:**
- Davies-Bouldin: DB = (1/K) sum_k max_{j!=k} (sigma_k+sigma_j)/d(mu_k,mu_j)
- Calinski-Harabasz: CH = (SSB/(K-1)) / (SSW/(N-K))

**Exercicios:**
- Exercicio 13.9: Avaliar clustering com diferentes metricas

---

### Capitulo 14: PCA e Reducao de Dimensionalidade

**Objetivo:** Reduzir dimensao preservando informacao, conectando com autovalores (Cap. 2), distancia (Cap. 7) e clustering (Cap. 13).

#### 14.1 A Maldicao da Dimensionalidade (Revisitada)
- 14.1.1 Por que reduzir dimensionalidade?
  - Esparsidade em altas dimensoes
  - Custo computacional
  - Ruido e redundancia
  - Visualizacao

**Sidebar:**
- Maldicao: volume cresce exponencialmente com d
- Solucoes: selecao de features, extracao de features (PCA)
- Conecta com KNN (Cap. 7) e regularizacao (Cap. 4)

#### 14.2 Principal Component Analysis (PCA)
- 14.2.1 Ideia central
  - Projetar dados na direcao de maxima variância
  - Componentes principais ortogonais
  - Conecta com autovalores e autovetores (Cap. 2)
- 14.2.2 Algoritmo PCA
  1. Centralizar dados: X_tilde = X - mu
  2. Matriz de covariancia: Sigma = (1/n) X_tilde^T X_tilde
  3. Autodecomposicao: Sigma v = lambda v
  4. Ordenar por autovalor (lambda_1 >= lambda_2 >= ...)
  5. Projetar: Z = X_tilde V_k (k primeiros componentes)
- 14.2.3 Explained Variance
  - lambda_k / sum_i lambda_i
  - Escolha de k: cumsum >= 0.95 (95% variância)

**Sidebar formulas:**
- PCA: Z = X_tilde V_k
- X_tilde = X - mu (centralizar!)
- Sigma = (1/n) X_tilde^T X_tilde (covariancia)
- Autodecomposicao: Sigma v_k = lambda_k v_k
- Explained variance: lambda_k / sum lambda_i
- Escolha k: cumsum(lambda_i/sum) >= 0.95

**Exercicios:**
- Exercicio 14.1: Implementar PCA do zero
- Exercicio 14.2: Escolher numero de componentes

#### 14.3 Reconstrucao e Perda
- 14.3.1 Reconstrucao dos dados
  - X_hat = Z V_k^T + mu
  - Erro de reconstrucao: ||X - X_hat||^2
- 14.3.2 Trade-off dimensionalidade vs perda
  - Mais componentes: mais informacao preservada
  - Menos componentes: mais compressao

**Sidebar formulas:**
- Reconstrucao: X_hat = Z V_k^T + mu
- Erro: ||X - X_hat||^2 = sum_{k+1}^d lambda_k
- Trade-off: mais componentes = menos perda

**Exercicios:**
- Exercicio 14.3: Reconstruir dados e calcular erro
- Exercicio 14.4: Plotar variância explicada acumulada

#### 14.4 LDA (Linear Discriminant Analysis)
- 14.4.1 PCA vs LDA
  - PCA: nao-supervisionado (maximiza variancia)
  - LDA: supervisionado (maximiza separabilidade)
- 14.4.2 Algoritmo LDA
  - Maximizar razao S_B/S_W (between/within scatter)
  - Projetar em K-1 dimensoes (K classes)
- 14.4.3 Quando usar PCA vs LDA

**Sidebar formulas:**
- LDA: max tr(V^T S_B V) / tr(V^T S_W V)
- S_B = between-class scatter
- S_W = within-class scatter
- LDA: supervisionado, PCA: nao-supervisionado

**Exercicios:**
- Exercicio 14.5: Comparar PCA vs LDA
- Exercicio 14.6: Visualizar dados com PCA e LDA

#### 14.5 Outras Tecnicas
- 14.5.1 Selecao de Features
  - Filter: correlacao, chi-quadrado, Fisher score
  - Wrapper: forward/backward selection, RFE
  - Embedded: Lasso, Random Forest importances
- 14.5.2 t-SNE
  - Nao-linear, para visualizacao
  - Preserva vizinhanca local
- 14.5.3 UMAP
  - Preserva estrutura local e global
  - Mais rapido que t-SNE

**Sidebar:**
- Filter: rapido, independente do modelo
- Wrapper: lento, melhor performance
- Embedded: compromisso entre velocidade e qualidade
- t-SNE: visualizacao 2D/3D, nao para features
- UMAP: mais rapido, preserva estrutura global

**Exercicios:**
- Exercicio 14.7: Selecionar features com Fisher score
- Exercicio 14.8: Visualizar dados com t-SNE

---

## PARTE VI: PROJETO DE ML E MLOPS

### Capitulo 15: Projeto Completo de Machine Learning

**Objetivo:** Integrar todos os conceitos em um pipeline completo, conectando com cada capitulo anterior.

#### 15.1 Pipeline de ML
- 15.1.1 Entender o problema
  - Definir objetivo e metricas
  - Que tipo de aprendizagem? (Cap. 1)
- 15.1.2 Preparacao dos dados
  - Limpeza e preprocessamento (Cap. 3.6)
  - Feature engineering
  - Normalizacao (Cap. 3.6)
- 15.1.3 Modelagem
  - Escolha de algoritmos (Cap. 3-14)
  - Tuning de hiperparametros (Cap. 9.4)
- 15.1.4 Avaliacao
  - Metricas (Cap. 9)
  - Validacao cruzada (Cap. 9.4)
- 15.1.5 Deploy
  - Serializacao do modelo
  - Monitoramento

**Sidebar:**
- Pipeline: dados -> preprocessamento -> modelo -> avaliacao -> deploy
- Cada etapa conecta com capitulos anteriores
- Erro mais comum: data leakage (normalizar antes do split)

#### 15.2 Nested K-Fold Cross-Validation
- 15.2.1 Loop externo: avaliacao
- 15.2.2 Loop interno: selecao de hiperparametros
- 15.2.3 Estimativa nao-enviesada

**Sidebar formulas:**
- Outer: K1 folds para avaliacao
- Inner: K2 folds para tuning
- Resultado: media +- desvio das K1 metricas externas

**Exercicios:**
- Exercicio 15.1: Implementar Nested CV completo

#### 15.3 Boas Praticas
- 15.3.1 Reprodutibilidade
  - Seeds, versionamento, containerizacao
- 15.3.2 Data leakage
  - Normalizar apos o split!
  - Nao usar dados de teste para selecionar features
- 15.3.3 MLOps
  - CI/CD para ML
  - Model registry
  - Monitoring: data drift, concept drift

**Sidebar:**
- Seed: fixar aleatoriedade
- Data leakage: usar informacao do teste no treino
- MLOps: devops para ML

**Exercicios:**
- Exercicio 15.2: Identificar e corrigir data leakage
- Exercicio 15.3: Pipeline reprodutivel

---

## APENDICES

### Apendice A: Exercicios de NumPy (Lista 0)

**Integracao com Cap. 2 (Fundamentos Matematicos)**

- A.1 Normalizacao Z-score
  - Calculo de media e desvio padrao
  - Normalizacao e desnormalizacao
  - Funcoes lambda: calc_mean, calc_std, normalize, denormalize
- A.2 Metodo de Newton-Raphson
  - Encontrar raizes de funcoes
  - Aplicacao: calcular sqrt(2)
  - Convergencia quadratica
- A.3 Regressao Linear com OLS (NumPy)
  - Operacoes matriciais
  - w* = (X^T X)^{-1} X^T y
  - Calculo de RMSE
- A.4 Regressao Linear com Gradient Descent (NumPy)
  - Normalizacao de dados
  - Curva de aprendizagem
  - Taxa de aprendizado: alpha = 0.1
- A.5 Regressao Linear com SGD (NumPy)
  - Atualizacao estocastica
  - Convergencia com oscilacoes
  - Comparacao com GD

**Sidebar formulas:**
- Z-score: Z = (X - mu) / sigma
- Newton-Raphson: x_{n+1} = x_n - f(x_n)/f'(x_n)
- OLS: w* = (X^T X)^{-1} X^T y
- GD: w <- w + alpha * (1/N) * X^T(y - Xw)
- SGD: w <- w + alpha * e_i * x_i

### Apendice B: Lista 1 - Regressao Linear

**Integracao com Cap. 3 (Regressao Linear) e Cap. 4 (Polinomial/Regularizacao)**

- B.1 Questao 1a: OLS (Solucao Analitica)
  - Dataset: artificial1d.csv
  - Implementacao: w* = (X^T X)^{-1} X^T y
  - Grafico: scatter + reta de regressao
- B.2 Questao 1b: Gradient Descent
  - Dataset: artificial1d.csv
  - Curva de aprendizagem
  - Parametros: alpha = 0.1, epocas = 1000
- B.3 Questao 1c: SGD
  - Dataset: artificial1d.csv
  - Comparacao com GD
  - Parametros: alpha = 0.01, epocas = 100
- B.4 Questao 2: Regressao Polinomial
  - Dataset: california.csv (1000 amostras, 8 features)
  - 13 modelos polinomiais (graus 1-13)
  - OLS sem regularizacao
  - Ridge com lambda = 0.01
  - Analise de overfitting/underfitting
  - Graficos comparativos: RMSE treino vs teste por grau
  - Comparacao OLS vs Ridge

**Sidebar formulas:**
- OLS: w* = (X^T X)^{-1} X^T y
- Ridge: w* = (lambda*I + X^T X)^{-1} X^T y
- RMSE_treino vs RMSE_teste por grau polinomial
- Normalizacao: MinMaxScaler para X, StandardScaler para y
- Data leakage: calcular mu e sigma apenas no treino!

### Apendice C: Lista 2 - Classificacao

**Integracao com Cap. 5 (Regressao Logistica) e Cap. 6 (Classificadores Estatisticos)**

- C.1 Questao 1a: Regressao Logistica (Classificacao Binaria)
  - Dataset: breastcancer.csv (569 amostras, 30 features, 2 classes)
  - 10-fold cross-validation
  - Metricas: acuracia global (media +- desvio), acuracia por classe
- C.2 Questao 1b: GDA - Analise do Discriminante Gaussiano
  - Dataset: breastcancer.csv
  - 10-fold cross-validation
  - Comparacao com Regressao Logistica
- C.3 Questao 1c: Naive Bayes Gaussiano
  - Dataset: breastcancer.csv
  - 10-fold cross-validation
  - Comparacao generativo vs discriminativo
- C.4 Questao 2a: Softmax Regression (Classificacao Multiclasse)
  - Dataset: vehicle.csv (846 amostras, 18 features, 4 classes)
  - 10-fold cross-validation
  - One-vs-All
- C.5 Questao 2b: GDA Multiclasse
  - Dataset: vehicle.csv
  - 10-fold cross-validation
  - Discriminante Linear e Quadratico
- C.6 Questao 2c: Naive Bayes Gaussiano Multiclasse
  - Dataset: vehicle.csv
  - 10-fold cross-validation
  - Log-probabilidades para estabilidade numerica

**Sidebar formulas:**
- K-Fold CV: dividir em K folds, treinar em K-1, testar em 1
- Logistic: P(y=1|x) = sigma(w^T x)
- Softmax: P(y=k|x) = exp(w_k^T x) / sum_j exp(w_j^T x)
- GDA: P(y=k|x) prop P(x|y=k) * P(y=k)
- Naive Bayes: P(y=k|x) prop P(y=k) * prod_i P(x_i|y=k)
- Acuracia global: media +- desvio das K folds

### Apendice D: Implementacoes em NumPy (Referencia Rapida)

- D.1 Regressao (OLS, GD, SGD, Ridge)
- D.2 Classificacao (Logistica, Softmax, GDA, Naive Bayes)
- D.3 Normalizacao (Z-score, MinMax, Standard)
- D.4 Metricas (RMSE, MSE, Acuracia, Precision, Recall, F1)
- D.5 Cross-Validation (K-Fold, Stratified)

### Apendice E: Datasets Utilizados

- E.1 artificial1d.csv (Regressao Linear)
- E.2 california.csv (California Housing - Regressao Polinomial)
- E.3 breastcancer.csv (Wisconsin Breast Cancer - Classificacao Binaria)
- E.4 vehicle.csv (Vehicle Silhouettes - Classificacao Multiclasse)

### Apendice F: Funcoes Lambda Consolidadas

- F.1 Regressao Linear
  - ols_fit = lambda X, y: np.linalg.solve(X.T @ X, X.T @ y)
  - ridge_fit = lambda X, y, lam: np.linalg.solve(lam*np.eye(X.shape[1]) + X.T@X, X.T@y)
  - predict = lambda X, w: X @ w
  - calc_rmse = lambda y_true, y_pred: np.sqrt(np.mean((y_true - y_pred)**2))
- F.2 Classificacao
  - sigmoid = lambda z: 1 / (1 + np.exp(-z))
  - softmax = lambda Z: np.exp(Z) / np.sum(np.exp(Z), axis=1, keepdims=True)
  - calc_accuracy = lambda y_true, y_pred: np.mean(y_true == y_pred)
- F.3 Normalizacao
  - normalize = lambda X, mu, sigma: (X - mu) / sigma
  - minmax_scale = lambda X, X_min, X_max: (X - X_min) / (X_max - X_min)
- F.4 Cross-Validation
  - create_folds = lambda n, k, seed: np.array_split(np.random.RandomState(seed).permutation(n), k)

### Apendice G: Referencias e Leitura Adicional

- G.1 Bishop (2006) - Capitulos relevantes por topico
- G.2 Mathematics for ML - Capitulos relevantes por topico
- G.3 Murphy (2022) - Capitulos relevantes por topico

---

## INDICES FINAIS
- Bibliografia
- Indice Remissivo
- Lista de Simbolos e Notacoes
- Guia Rapido de Formulas

---

## GUIA DE ESTUDO RAPIDO
- Formulas-chave por capitulo
- Fluxograma: qual algoritmo usar?
- Checklist pre-prova
- Tabela de comparacao de modelos