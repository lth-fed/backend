#let data = json("./data.json")
#let lang = data.language
#let sv = lang == "sv"

#set text(lang: lang)
#set document(title: if sv [Försäljningsrapport - #data.activity_name] else [Sales report - #data.activity_name])
#set par(justify: true)
#set page("a4")

#show table: set table(
  inset: 4pt,
  fill: (_, y) => if calc.odd(y) { luma(90%) } else { white },
  stroke: none,
)

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

#if data.creator_logo_format != none [#place(right)[
  #image(
    read("image", encoding: none),
    format: data.creator_logo_format,
    width: 8em,
  )
]] else []

= #if sv [Försäljningsrapport] else [Sales report]

#if sv [Aktivitet] else [Activity]: #data.activity_name \
#if sv [Skapare av aktiviteten] else [Activity creator]: #data.creator_name

#let total_sales = data.per_object.map(obj => obj.price * obj.number).sum(default: 0)
#let total = if sv [Totalt] else [Total]

== #if sv [Total försäljning och avgifter] else [Total sales and fees]

#table(
  columns: (auto, auto),
  align: (left, right),
  table.header(if sv [Kategori] else [Category], table.vline(), if sv [Belopp (inkl. moms)] else [Amount (incl. VAT)]),
  table.hline(),
  if sv [Total försäljning] else [Total sales], format_currency(total_sales),
  if sv [Totala avgifter] else [Total fees], [-#format_currency(data.fees)],
  if sv [Totala avgifter från externa tjänster] else [Total fees from external services],
  [-#format_currency(data.fees_external)],
  table.hline(),
  total, format_currency(total_sales - data.fees - data.fees_external),
)

== #if sv [Försäljning per objekt] else []

#table(
  columns: (auto, auto, auto, auto, auto),
  align: (left, left, right, right, right),
  table.header(
    if sv [Typ] else [Kind],
    table.vline(),
    if sv [Namn] else [Name],
    table.vline(),
    if sv [Antal] else [Number sold],
    table.vline(),
    if sv [Styckpris] else [Price per],
    table.vline(),
    if sv [Belopp (inkl. moms)] else [Total (incl. VAT)],
  ),
  table.hline(),
  ..data
    .per_object
    .map(obj => (
      if obj.kind == "ticket" [#if sv [Biljett] else [Ticket]] else if obj.kind
        == "option" [#if sv [Tillval] else [Addon]] else [#if sv [Extern] else [External]],
      [#obj.name],
      str(obj.number),
      format_currency(obj.price),
      format_currency(obj.number * obj.price),
    ))
    .flatten(),
  table.hline(),
  total, [], [], [], format_currency(data.per_object.map(obj => obj.price * obj.number).sum(default: 0)),
)

== #if sv [Försäljning per alkoholkategori] else [Sales per alcohol category]

#table(
  columns: (auto, auto),
  align: (left, right),
  table.header(if sv [Kategori] else [Category], table.vline(), if sv [Belopp (inkl. moms)] else [Amount (incl. VAT)]),
  table.hline(),
  ..data
    .per_alcohol_category
    .map(obj => (
      if obj.name == "null" [#if sv [Icke-alkohol] else [Non-alcohol]] else [#obj.name],
      format_currency(obj.amount),
    ))
    .flatten(),
  table.hline(),
  total, format_currency(data.per_alcohol_category.map(obj => obj.amount).sum(default: 0)),
)

#for index in range(data.receipt_count) {
  pagebreak()
  image(
    read("transaction-receipt-" + str(index) + ".pdf", encoding: none),
    format: "pdf",
    width: 100%,
    height: 100%,
    fit: "contain",
  )
}
