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

#import "format.typ": format_currency

#if data.creator_logo_format != none [#place(right)[
  #image(
    read("image", encoding: none),
    format: data.creator_logo_format,
    width: 8em,
  )
]] else []

= #if sv [Försäljningsrapport] else [Sales report]

#if data.is_group [
  #if sv [Period] else [Period]: #data.activity_name \
  #if sv [Grupp] else [Group]: #data.creator_name \
] else [
  #if sv [Aktivitet] else [Activity]: #data.activity_name \
  #if sv [Skapare av aktiviteten] else [Activity creator]: #data.creator_name \
]
#if data.exported_at != none [
  #if sv [Bokföringsdatum] else [Date of bookkeeping]: #data.exported_at
]

#let total_sales = data.per_object.map(obj => obj.price * obj.number).sum(default: 0)
#let total_refunds = data.per_refund.map(obj => obj.price * obj.number).sum(default: 0)
#let total = if sv [Totalt] else [Total]

== #if sv [Total försäljning och avgifter] else [Total sales and fees]

#table(
  columns: (auto, auto),
  align: (left, right),
  table.header(if sv [Kategori] else [Category], table.vline(), if sv [Belopp (inkl. moms)] else [Amount (incl. VAT)]),
  table.hline(),
  if sv [Total försäljning] else [Total sales], format_currency(total_sales),
  if sv [Totala avgifter] else [Total fees], [-#format_currency(data.fees)],
  ..(if data.fees_refund != 0 { (if sv [Totala återbetalningsavgifter] else [Total refund fees], [-#format_currency(data.fees_refund)]) } else { () }),
  ..(if total_refunds != 0 { (if sv [Totala återbetalningar] else [Total refunds], [-#format_currency(total_refunds)]) } else { () }),
  if sv [Totala avgifter från externa tjänster] else [Total fees from external services],
  [-#format_currency(data.fees_external)],
  table.hline(),
  total, format_currency(total_sales - data.fees - data.fees_refund - total_refunds - data.fees_external),
)

== #if sv [Försäljning per objekt] else [Sales per item]
#table(
  columns: (auto, auto, auto, auto),
  align: (left, right, right, right),
  table.header(
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
      [#obj.name],
      str(obj.number),
      format_currency(obj.price),
      format_currency(obj.number * obj.price),
    ))
    .flatten(),
  table.hline(),
  total, [], [], format_currency(data.per_object.map(obj => obj.price * obj.number).sum(default: 0)),
)

#if data.per_refund.len() > 0 [
  == #if sv [Återbetalningar per objekt] else [Refunds per item]

  #table(
    columns: (auto, auto, auto, auto),
    align: (left, right, right, right),
    table.header(
      if sv [Namn] else [Name],
      table.vline(),
      if sv [Antal] else [Number refunded],
      table.vline(),
      if sv [Styckpris] else [Price per],
      table.vline(),
      if sv [Belopp (inkl. moms)] else [Amount (incl. VAT)],
    ),
    table.hline(),
    ..data
      .per_refund
      .map(obj => (
        [#obj.name],
        str(obj.number),
        format_currency(obj.price),
        format_currency(obj.number * obj.price),
      ))
      .flatten(),
    table.hline(),
    total, [], [], format_currency(total_refunds),
  )
]

== #if sv [Försäljning per alkoholkategori] else [Sales per alcohol category]

#table(
  columns: (auto, auto),
  align: (left, right),
  table.header(if sv [Kategori] else [Category], table.vline(), if sv [Belopp (inkl. moms)] else [Amount (incl. VAT)]),
  table.hline(),
  ..data
    .per_alcohol_category
    .map(obj => (
      if obj.name == "null" [#if sv [Biljetter / annat] else [Tickets / other]] else [#obj.name],
      format_currency(obj.amount),
    ))
    .flatten(),
  table.hline(),
  total, format_currency(data.per_alcohol_category.map(obj => obj.amount).sum(default: 0)),
)
