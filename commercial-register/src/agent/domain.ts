import type {
  CommercialMetrics,
  MoneyInput,
  PurchasedPackageUpdate,
  TwentyRecord,
} from './types';

type TwentyMoney = {
  amountMicros?: string | number | null;
  currencyCode?: string | null;
};

type PackageForMrr = Partial<PurchasedPackageUpdate> & TwentyRecord;

export function moneyToTwenty(money: MoneyInput): TwentyMoney {
  return {
    amountMicros: String(Math.round(money.amount * 1_000_000)),
    currencyCode: money.currencyCode.toUpperCase(),
  };
}

export function usdToTwenty(amount: number): TwentyMoney {
  return moneyToTwenty({ amount, currencyCode: 'USD' });
}

export function moneyAmount(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (!isObject(value)) return undefined;
  if (typeof value.amount === 'number' && Number.isFinite(value.amount)) {
    return value.amount;
  }
  const micros = value.amountMicros;
  if (
    (typeof micros === 'string' || typeof micros === 'number') &&
    Number.isFinite(Number(micros))
  ) {
    return Number(micros) / 1_000_000;
  }
  return undefined;
}

export function moneyCurrency(value: unknown): string | undefined {
  if (!isObject(value)) return undefined;
  const currency = value.currencyCode;
  return typeof currency === 'string' ? currency.toUpperCase() : undefined;
}

export function monthlyRecurringRevenueUsd(
  purchasedPackage: Partial<PurchasedPackageUpdate> | TwentyRecord,
  at = new Date(),
): number {
  if (
    purchasedPackage.status !== 'ACTIVE' ||
    purchasedPackage.priceBasis !== 'RECURRING' ||
    !isEffectiveAt(purchasedPackage, at)
  ) {
    return 0;
  }

  return normalizedMonthlyRecurringRevenueUsd(purchasedPackage);
}

export function normalizedMonthlyRecurringRevenueUsd(
  purchasedPackage: Partial<PurchasedPackageUpdate> | TwentyRecord,
): number {
  if (purchasedPackage.priceBasis !== 'RECURRING') return 0;

  const price = moneyAmount(purchasedPackage.price);
  const currency = moneyCurrency(purchasedPackage.price);
  if (price === undefined || currency === undefined) return 0;

  if (currency !== 'USD') {
    const sourcedMonthlyPriceUsd = moneyAmount(
      purchasedPackage.sourcedMonthlyPriceUsd,
    );
    return sourcedMonthlyPriceUsd === undefined
      ? 0
      : roundMoney(sourcedMonthlyPriceUsd);
  }

  const divisor =
    purchasedPackage.billingCadence === 'MONTHLY'
      ? 1
      : purchasedPackage.billingCadence === 'QUARTERLY'
        ? 3
        : purchasedPackage.billingCadence === 'ANNUAL'
          ? 12
          : undefined;
  return divisor === undefined ? 0 : roundMoney(price / divisor);
}

export function deriveMetrics(
  packages: TwentyRecord[],
  offeringLines: TwentyRecord[],
  incomingPayments: TwentyRecord[],
  accounts: TwentyRecord[] = [],
  at = new Date(),
): CommercialMetrics {
  const currentMrrUsd = roundMoney(
    packages.reduce(
      (sum, purchasedPackage) =>
        sum + monthlyRecurringRevenueUsd(purchasedPackage as PackageForMrr, at),
      0,
    ),
  );
  const paymentValues = incomingPayments.map(paymentCashUsd);
  const lifetimeNetCashUsd =
    accounts.some((account) => account.cashHistoryReconciled !== true) ||
    paymentValues.some((value) => value === null)
    ? null
    : roundMoney(
        paymentValues.reduce<number>((sum, value) => sum + (value ?? 0), 0),
      );

  return {
    currentMrrUsd,
    lifetimeNetCashUsd,
    isCurrentCustomer: offeringLines.some((line) => line.status === 'ACTIVE'),
  };
}

function paymentCashUsd(payment: TwentyRecord): number | null {
  if (payment.status !== 'RECEIVED') return 0;
  if (
    typeof payment.assetCode === 'string' &&
    payment.assetCode.toUpperCase() === 'USD' &&
    typeof payment.nativeAmount === 'number'
  ) {
    return payment.nativeAmount;
  }
  return moneyAmount(payment.reportingValueUsd) ?? null;
}

function isEffectiveAt(
  purchasedPackage: Partial<PurchasedPackageUpdate> | TwentyRecord,
  at: Date,
): boolean {
  const day = at.toISOString().slice(0, 10);
  const from = purchasedPackage.effectiveFrom;
  const to = purchasedPackage.effectiveTo;
  return (
    (typeof from !== 'string' || from <= day) &&
    (typeof to !== 'string' || to >= day)
  );
}

function roundMoney(value: number): number {
  return Math.round((value + Number.EPSILON) * 100) / 100;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
