//! `describe`: command list without a name, syntax and examples with one.
//! Tool descriptions stay short; details live here (design.md "MCP tool").

use dsl::names;

const INDEX: &str = "\
쓸 수 있는 명령 (1단계). describe(name)으로 자세히:
  define   함수 정의·수정 (같은 이름에 다시 쓰면 수정)
  del      정의 삭제 (부르는 함수도 함께 지워짐)
  식       연산자, 우선순위, 괄호
  이름     이름 규칙, 그리스 문자, 첨자
  함수     기본 함수 목록
  sum      sum / prod
  상수     슬라이더 상수 규칙
  오류     오류 형식
아직 없음: constant:, highlight, req, scene, step, 행렬, split, 매개변수·극좌표·음함수";

const DEFINE: &str = "\
define: 블록 안에 한 줄에 정의 하나 (스페이스 4칸 들여쓰기).
  define:
      f(x; a, b) -> y: a*x^2 + b*x + 1
      g(x; a, b) -> y: f(x) + 1
- 괄호 안 ';' 앞은 입력 변수, 뒤는 상수(슬라이더). '->' 뒤는 출력 축. 1단계는 f(x) -> y만.
- 식에 쓰는 상수는 전부 ';' 뒤에 적음. 부르는 함수가 쓰는 상수도 적어야 함 (g가 f를 부르면 a, b).
- 같은 이름을 다시 정의하면 수정. f에 상수가 추가·제거되면 부르는 함수에 서버가 자동으로 반영하고 응답에 알림.
- 자기 자신이나 서로를 부르는 정의(재귀)는 안 됨.";

const DEL: &str = "\
del f        f를 지움
del f, g     여러 개
- f를 부르는 함수도 연쇄로 지워지고 응답에 모두 나옴.
- 상수는 직접 지우지 않음. 쓰는 함수가 없어지면 자동으로 사라짐.";

const EXPR: &str = "\
우선순위 (위가 먼저): 호출 f(x) > ! > ^ (오른쪽부터) > 단항 - > * / > + - > 비교 > not > and > or
- 곱셈 '*' 필수: 2*x (2x는 오류). 나눗셈 a/b*c = (a/b)*c.
- -x^2 = -(x^2). ^ 오른쪽은 숫자·이름·괄호 하나: x^(n+1), 2^(-1), x^{1/3}.
- () 보이는 괄호, {} 안 보이는 묶음. 절댓값 abs(x), 근호 sqrt(x).
- 함수 거듭제곱은 sin(x)^2 (sin^2(x)는 오류). 숫자 1.5, 0.5, 1e-3 (.5, 5.는 오류).
- 같음 ==, 같지 않음 not (a == b). 비교는 같은 방향으로 이어 씀: 0 <= x < 5.";

const NAMES: &str = "\
- 이름은 글자로 시작, 숫자 포함 가능, 한글 가능: T1, 넓이.
- 그리스 문자·특수 문자는 LaTeX 이름: \\alpha \\theta \\Delta (θ 같은 유니코드는 오류).
- 예약어: \\pi \\e (\\i 복소수, \\infty는 1단계에서 값으로 못 씀).
- 아래첨자는 중괄호 필수: x_{1}, a_{n} (x_1은 오류). 장식: \\bar{x} \\hat{x} \\vec{v} \\tilde{x} \\dot{x}.";

const SUM: &str = "\
sum(식; 아래 <= 인덱스 <= 위), prod(...)도 같은 모양.
  f(x; n) -> y: sum(x^k/k!; 0 <= k <= n)
- 인덱스는 정수. 범위에 쓰인 상수는 정수 슬라이더가 됨.
- 범위에 x를 쓰면 (k <= x) 정수 x에서만 값이 있어 점으로 그려짐.";

const CONSTANTS: &str = "\
- 상수는 전역, 이름이 같으면 슬라이더 공유. 처음 쓰이면 자동 생성 (값 1, 범위 -10..10; 정수는 5, 0..20).
- 사용자가 슬라이더 값과 범위를 바꿀 수 있음. 현재 값은 get_state로 확인.
- 조건 선언(constant: 블록)은 2단계.";

const ERRORS: &str = "\
오류 한 줄 형식: 줄:열 [오류 코드] 설명 → 후보
  3:12 [E-unicode] 'θ'는 쓸 수 없음 → \\theta
- 한 요청은 전부 적용되거나 전부 거절됨. 독립적인 오류는 한 번에 모두 보고됨.
- [E-unsupported]는 다음 단계 기능.";

pub fn describe(name: Option<&str>) -> String {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
        return INDEX.into();
    };
    match name {
        "define" | "정의" => DEFINE.into(),
        "del" | "삭제" => DEL.into(),
        "식" | "연산" | "expr" | "operators" => EXPR.into(),
        "이름" | "names" => NAMES.into(),
        "sum" | "prod" => SUM.into(),
        "상수" | "constant" | "slider" => CONSTANTS.into(),
        "오류" | "errors" => ERRORS.into(),
        "함수" | "functions" | "builtins" => {
            let list: Vec<String> = names::BUILTINS
                .iter()
                .map(|(n, a)| if a[0] == 1 { format!("{n}(x)") } else if *n == "log" { "log(x, b)".into() } else { format!("{n}(a, m)") })
                .collect();
            format!("기본 함수: {}\n- 밑 없는 log는 없음: ln, log10, log2, log(x, b).\n- 삼각함수는 라디안, 도는 deg(30).", list.join(", "))
        }
        other => format!("'{other}'에 대한 설명이 없음.\n{INDEX}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_and_topics() {
        assert!(describe(None).starts_with("쓸 수 있는 명령"));
        assert!(describe(Some("define")).contains("f(x; a, b) -> y"));
        assert!(describe(Some("함수")).contains("log(x, b)"));
        assert!(describe(Some("??")).contains("설명이 없음"));
    }

    /// Examples in the help text must be valid DSL.
    #[test]
    fn examples_parse() {
        let src = "define:\n    f(x; a, b) -> y: a*x^2 + b*x + 1\n    g(x; a, b) -> y: f(x) + 1\n    s(x; n) -> y: sum(x^k/k!; 0 <= k <= n)\n";
        dsl::interp::apply(&Default::default(), src, &Default::default(), 1).unwrap();
    }
}
