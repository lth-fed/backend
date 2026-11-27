#import "format.typ": format_currency

#let data = json("./data.json")
#let lang = data.language
#let sv = lang == "sv"

#set text(lang: lang)
#set document(title: if sv [Lagerrapport - #data.group_name] else [Stock report - #data.group_name])
#set par(justify: true)
#set page("a4")

#show table: set table(
  inset: 4pt,
  fill: (_, y) => if calc.odd(y) { luma(90%) } else { white },
  stroke: none,
)

#let kind_label(kind) = {
  if kind == "take-out" [#if sv [Uttag] else [Take-out]]
  else if kind == "put-back" [#if sv [Återläggning] else [Put-back]]
  else if kind == "purchase" [#if sv [Inköp] else [Purchase]]
  else if kind == "refund" [#if sv [Retur till leverantör] else [Supplier return]]
  else if kind == "sales" [#if sv [Försäljning] else [Sale]]
  else if kind == "sales-refund" [#if sv [Återbetalning] else [Sale refund]]
  else if kind == "inventory" [#if sv [Inventering] else [Stocktake]]
  else [#if sv [Svinn] else [Loss]]
}

= #if sv [Lagerrapport] else [Stock report]

#if sv [Grupp] else [Group]: #data.group_name \
#if sv [Period] else [Period]: #data.period \
#if data.exported_at != none [
  #if sv [Exporterad] else [Exported]: #data.exported_at
]

== #if sv [Rörelser] else [Movements]

#table(
  columns: (auto, auto, 1fr, 1fr, auto, auto, auto),
  align: (left, left, left, left, right, left, right),
  table.header(
    if sv [Datum] else [Date],
    if sv [Typ] else [Type],
    if sv [Namn] else [Name],
    if sv [Aktivitet] else [Activity],
    if sv [Antal] else [Amount],
    if sv [Produkt] else [Product],
    if sv [Värde] else [Value],
  ),
  table.hline(),
  ..data.movements.map(row => (
    row.time,
    kind_label(row.kind),
    row.name,
    row.activity_name,
    str(row.amount),
    row.product_name,
    if row.value != none [#format_currency(row.value)] else [–],
  )).flatten(),
)

#if data.movements.len() == 0 [
  #if sv [Inga rörelser under perioden.] else [No movements in the period.]
]

== #if sv [Summering per produkt] else [Summary per product]

#table(
  columns: (1fr, auto, auto),
  align: (left, right, right),
  table.header(if sv [Produkt] else [Product], if sv [Netto antal] else [Net amount], if sv [Netto värde] else [Net value]),
  table.hline(),
  ..data.per_product.map(row => (
    row.product_name,
    str(row.net_amount),
    format_currency(row.net_value),
  )).flatten(),
)
