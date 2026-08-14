import { getFortnoxApiClient } from '$lib/fortnox';
import { getVoucher, getVoucherWithFortnoxIdents, setVoucherFortnoxIdent } from '$lib/prisma';
import { config } from '@/config.private';
import type { RequestHandler } from './$types';
import { manageError } from '@/helpers/fortnox.helpers';
import type { VoucherFortnoxType, VoucherType } from '@/models/voucher.models';

const buildHakeFormData = (voucher: VoucherType, ident: VoucherFortnoxType) => {
	const fullVerNr = `${ident.series}${ident.number}`;
	const formData = new FormData();
	formData.append('voucher_nbr', fullVerNr);
	formData.append('date_turned_in', voucher.dateTurnedIn.toISOString().slice(0, 10));
	const name =
		voucher.driversExpense?.name ??
		voucher.expenseClaim?.name ??
		voucher.expenseReport?.name ??
		voucher.reference;
	if (name) {
		formData.append('name', name);
	}
	const dateOfPurchase = voucher.expenseClaim?.receiptDate ?? voucher.expenseReport?.receiptDate;
	if (dateOfPurchase) {
		formData.append('date_of_purchase', dateOfPurchase.toISOString().slice(0, 10));
	}
	if (voucher.approvedAt) {
		formData.append('uordf_date', voucher.approvedAt.toISOString().slice(0, 10));
	}

	formData.append('ssn', voucher.expenseClaim?.ssn ?? voucher.driversExpense?.ssn ?? '');
	formData.append('utskott', voucher.utskott);
	formData.append('proj', voucher.description);
	formData.append('uordf', voucher.approvedBy ?? '');
	formData.append('pm', voucher.accountedBy ?? '');
	formData.append('pm_date', new Date().toISOString().slice(0, 10));
	if (voucher.attachmentUrl) {
		formData.append('receiptUrl', voucher.attachmentUrl);
	}
	return formData;
};
const kvittohake = async (fetch: any, voucher: VoucherType, ident: VoucherFortnoxType) => {
	const formData = buildHakeFormData(voucher, ident);
	return await fetch(config.KVITTOHAKE_URL, {
		method: 'POST',
		body: formData
	});
};

export const GET: RequestHandler = async ({ fetch, params }) => {
	const { id } = params;
	const voucher = await getVoucherWithFortnoxIdents(id);
	if (!voucher) {
		return new Response('Verifikat existerar inte', { status: 400 });
	}
	if (!voucher.accounted) {
		return new Response('Verifikat har inte bokförts än', { status: 400 });
	}
	if (!voucher.ident) {
		return new Response(
			'Verifikatet har bokförts under en inkompatibel, tidigare version av kvittosidan',
			{ status: 400 }
		);
	}
	try {
		const pdf = await kvittohake(fetch, voucher, voucher.ident);
		if (!pdf.ok) {
			console.error(await pdf.text());
			return new Response('Kvittohake is big sad', { status: 400 });
		}
		return new Response(await pdf.blob(), {
			headers: { 'content-type': 'application/octet-stream' }
		});
	} catch (e) {
		console.error(e);
		return new Response('Kvittohake is big big sad', { status: 400 });
	}
};

export const POST: RequestHandler = async ({ fetch, params, request }) => {
	const { id } = params;
	const voucher = await getVoucher(id);
	if (!voucher) {
		return new Response('Verifikat existerar inte', { status: 400 });
	}

	if (voucher.accounted) {
		return new Response('Otillåtet att bokföra redan bokfört verifikat', { status: 400 });
	}

	if (!voucher.accountedBy) {
		return new Response('Saknar bokföringsanvändare', { status: 400 });
	}
	if (!voucher.approvedBy) {
		return new Response('Saknar godkännare', { status: 400 });
	}

	if (!voucher.transactionDate || isNaN(new Date(voucher.transactionDate).getTime())) {
		return new Response('Saknar transaktionsdatum', { status: 400 });
	}

	if (!voucher.label) {
		return new Response('Saknar etikett', { status: 400 });
	}

	const accounting = voucher.accounting;

	if (!accounting?.length) {
		return new Response('Ingen kontering på verifikat', { status: 400 });
	}

	if (accounting.length === 1) {
		return new Response('Kontering kan inte vara en rad lång', { status: 400 });
	}

	const fortnoxAccounting = accounting.map((a) => ({
		Account: a.account?.number ?? -1,
		CostCenter: a.profitCenter?.name,
		Credit: a.kreditInCents / 100,
		Debit: a.debetInCents / 100,
		Project: a.project?.number?.toString()
	}));

	if (fortnoxAccounting.some((a) => a.Account === -1)) {
		return new Response('Varje rad i konteringen måste specificera ett kontonummer', {
			status: 400
		});
	}
	let balance = 0;
	fortnoxAccounting.forEach((a) => (balance += a.Debit - a.Credit));
	balance = Math.round(balance * 100) / 100; //because floating point is retarded
	if (balance != 0) {
		return new Response('Kontering måste vara balanserad', { status: 400 });
	}

	const fortnoxApi = getFortnoxApiClient(fetch);

	const si = voucher.supplierInvoice;
	if (si) {
		if (!si.companyNumber) {
			return new Response('Leverantörsnummer saknas', { status: 400 });
		}
		let total = 0;
		accounting.forEach((a) => (total += a.kreditInCents));
		const { data, error } = await fortnoxApi.POST('/3/supplierinvoices', {
			body: {
				SupplierInvoice: {
					SupplierNumber: si.companyNumber,
					SupplierInvoiceRows: fortnoxAccounting.filter((a) => a.Account !== 2440),
					Total: (total / 100).toFixed(2),
					OCR: si.ocr,
					InvoiceNumber: si.invoiceNumber,
					DueDate: si.dueDate.toISOString().slice(0, 10)
				}
			}
		});

		if (!data || error) {
			return manageError(error);
		}
		const givenNumber = data.SupplierInvoice?.GivenNumber;
		if (voucher.attachmentUrl) {
			try {
				const response = await fetch(voucher.attachmentUrl);
				const receipt = await response.blob();
				if (receipt) {
					const formData = new FormData();
					const extension = voucher.attachmentUrl.split('.').pop() || 'pdf';
					formData.append(
						'file',
						receipt,
						`${new Date().toISOString().slice(0, 10)}attachment.${extension}`
					);
					const inboxRes = await fetch('https://api.fortnox.se/3/inbox', {
						method: 'POST',
						body: formData
					});
					if (!inboxRes.ok) {
						return new Response('Misslyckades att ladda upp bilaga till Fortnox inbox', {
							status: 201
						});
					}
					const fileId = ((await inboxRes.json()) as { File: { Id: string } }).File.Id;
					const res = await fortnoxApi.POST('/3/supplierinvoicefileconnections', {
						body: {
							SupplierInvoiceFileConnection: {
								FileId: fileId,
								SupplierInvoiceNumber: givenNumber
							}
						}
					});
					if (!res.data || res.error) {
						return manageError(res.error, 201);
					}
				}
			} catch {
				return new Response('Misslyckades att hämta bilaga för att ladda upp till Fortnox', {
					status: 201
				});
			}
		}

		return new Response('Verifikat skapat!', { status: 200 });
	}

	const { data, error } = await fortnoxApi.POST('/3/vouchers', {
		body: {
			Voucher: {
				Description: voucher.label.slice(0, 200).trim(),
				TransactionDate: voucher.transactionDate.toISOString().slice(0, 10),
				VoucherSeries: voucher.series,
				VoucherRows: fortnoxAccounting,
				Comments: voucher.comment?.slice(0, 1000).trim() || ''
			}
		}
	});

	if (!data || error) {
		return manageError(error);
	}

	if (!data.Voucher) {
		return new Response(
			'Oväntat returvärde från Fortnox. Kunde inte generera bilaga tillhörande verifikatet',
			{
				status: 201
			}
		);
	}

	const voucherUrl = data.Voucher['@url'];
	const verSeries = data.Voucher.VoucherSeries;
	const verNr = data.Voucher.VoucherNumber;
	const verYear = data.Voucher.Year;

	if (!voucherUrl || !verSeries || !verNr || !verYear) {
		return new Response(
			'Saknar returvärden från Fortnox. Kunde inte generera bilaga tillhörande verifikatet',
			{ status: 201 }
		);
	}

	const ident: VoucherFortnoxType = await setVoucherFortnoxIdent(
		voucher.id,
		verNr,
		verSeries,
		verYear,
		voucherUrl
	);

	const fullVerNr = `${verSeries}${verNr}`;

	const fortnoxAttachment = await kvittohake(fetch, voucher, ident);

	if (!fortnoxAttachment.ok) {
		return new Response(
			`${fullVerNr} misslyckades att generera bilaga för uppladdning till Fortnox`,
			{
				status: 201
			}
		);
	}

	const pdfBuffer = Buffer.from(await fortnoxAttachment.arrayBuffer());
	const inboxForm = new FormData();
	inboxForm.append(
		'file',
		new Blob([pdfBuffer], { type: 'application/pdf' }),
		verSeries + verNr + '.pdf'
	);
	const inboxRes = await fetch('https://api.fortnox.se/3/inbox?path=inbox_v', {
		method: 'POST',
		body: inboxForm
	});

	if (!inboxRes.ok) {
		return new Response(`${fullVerNr} misslyckades att ladda upp bilaga till Fortnox inbox`, {
			status: 201
		});
	}

	const fileId = ((await inboxRes.json()) as { File: { Id: string } }).File.Id;
	const res = await fortnoxApi.POST('/3/voucherfileconnections', {
		body: {
			VoucherFileConnection: {
				FileId: fileId,
				VoucherNumber: verNr.toString(),
				VoucherSeries: verSeries
			}
		}
	});

	if (!res.data || res.error) {
		return manageError(res.error, 201, `${fullVerNr} `);
	}

	return new Response(`Verifikat ${fullVerNr} skapat!`, { status: 200 });
};

