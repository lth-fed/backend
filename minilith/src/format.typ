#let sv = json("./data.json").language == "sv"

#let format_currency(number) = {
  let precision = 2
  assert(precision > 0)
  let s = str(calc.round(number / 100, digits: precision))
  let after_dot = s.find(regex("\..*"))
  if after_dot == none {
    s = s + "."
    after_dot = "."
  }
  for i in range(precision - after_dot.len() + 1) {
    s = s + "0"
  }

  if sv {
    s = s.replace(".", ",")
  }

  [#s SEK]
}
