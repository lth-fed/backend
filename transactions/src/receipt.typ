#let data = json("./data.json")
#let lang = data.language
#let sv = lang == "sv"

#set text(lang: lang)
#set document(title: if sv [Kvitto - #data.transaction_id] else [Receipt - #data.transaction_id])
#set par(justify: true)
#set page("a4")

#let format_currency(number) = {
  let precision = 2
  assert(precision > 0)
  let s = str(calc.round(number, digits: precision))
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
#let wares = data.wares.map(ware => {
  (
    if ware.name.starts-with("    ") [
      #grid(
        columns: (16pt, 1fr),
        [], ware.name.slice(4),
      )
    ] else { ware.name },
    [#str((ware.tax - 1) * 100)%],
    format_currency(ware.amount / ware.tax / 100),
    format_currency(ware.amount / 100),
  )
})

#grid(
  columns: (1fr, auto),
  align(horizon)[
    = #if sv [Kvitto] else [Receipt]

    #if sv [Ordernummer] else [Order reference]: #data.transaction_id \
    #if sv [Orderdatum] else [Date of purchase]: #data.purchase_date \
    #if sv [Betalsätt] else [Payment provider]: #data.provider \
    #if sv [Betalreferens] else [Payment reference]: #data.payment_reference \
    #if data.refund_reference != none [#if sv [Returreferens] else [Return reference]: #data.refund_reference]

    #if sv [Kund] else [Customer]: #data.customer_name
    #if sv [Kundnummer] else [Customer ID]: #data.customer_id
  ],
  if data.merchant_svg_icon != none [#image(bytes(data.merchant_svg_icon), format: "svg", width: 150pt)] else [],
)

#h(1cm)

#let tbl-vline() = [#table.vline(stroke: luma(80%))]

#align(center, table(
  columns: (auto, auto, auto, auto),
  align: (left, right, right, right),
  inset: 4pt,
  fill: (_, y) => if calc.odd(y) { luma(90%) } else { white },
  stroke: none,
  table.header(
    if sv [Produktnamn] else [Product name], tbl-vline(), if sv [Moms] else [VAT], tbl-vline(),
    if sv [Belopp exkl. moms] else [Cost (excl. VAT)], tbl-vline(), if sv [Belopp inkl. moms] else [Cost (incl. VAT)],
  ),
  table.hline(),
  ..wares.flatten(),
  table.hline(),
  if sv [Total] else [Total], [],
  format_currency(data.wares.fold(0, (acc, ware) => acc + ware.amount / 100 / ware.tax)),
  format_currency(data.wares.fold(0, (acc, ware) => acc + ware.amount / 100)),
))

#align(bottom)[
  #line()
  #align(top, stack(
    dir: ltr,
    spacing: 1cm,
    [
      #data.merchant_name \
      #data.merchant_address
    ],
    [
      #if sv [Org.nr] else [Org.nr]: #data.merchant_org_id \
      #if sv [E-post] else [Email]: #data.merchant_email
    ],
  ))
]
