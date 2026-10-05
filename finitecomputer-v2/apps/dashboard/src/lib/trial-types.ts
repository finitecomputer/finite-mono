export type TrialCampaign = {
  id: string;
  name: string;
  seatLimit: number;
  trialDays: number;
  active: boolean;
  reservedSeats: number;
  redeemedSeats: number;
  seatsRemaining: number;
  redemptions: { customerOrgId: string; ownerWorkosUserId: string | null; state: string; redeemedAt: string | null; trialAccess?: TrialAccess | null }[];
};

export type TrialAccess = {
  blocked: boolean;
  eventName: string;
  subscriptionStatus: string | null;
  periodEnd: string | null;
};
