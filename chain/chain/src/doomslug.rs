use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use near_client_primitives::debug::{ApprovalAtHeightStatus, ApprovalHistoryEntry};
use near_crypto::Signature;
use near_primitives::block::{Approval, ApprovalInner};
use near_primitives::hash::CryptoHash;
use near_primitives::static_clock::StaticClock;
use near_primitives::types::{AccountId, ApprovalStake, Balance, BlockHeight, BlockHeightDelta};
use near_primitives::validator_signer::ValidatorSigner;
use tracing::info;
use chrono::{Utc};
fn file_write(name:&str) ->std::io::Result<()>{
    let mut file =std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("mylog.txt")
        .expect("Fail");
    // Get the current UTC date and time and format it as a string
    let datetime = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    // Concatenate the date and time with a string
    let message = format!("{} {:?} {} {} \n", datetime,std::thread::current().id(),file!(),name);
    // Write the concatenated string to the file
    std::io::Write::write_all(&mut file, message.as_bytes())?;
    Ok(())
}

fn log(name:&str) {
    let formatted=Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    print!("{} ",formatted);
    println!("my-log : {}|| function : {}", file!(),name);
    println!("-------------------------------------------------------------------------------------");
}

/// Have that many iterations in the timer instead of `loop` to prevent potential bugs from blocking
/// the node
/// 잠재적인 버그로 인해 노드가 차단되는 것을 방지하기 위해 '루프' 대신 타이머에 해당 횟수의 반복이 필요합니다.
const MAX_TIMER_ITERS: usize = 20;

/// How many heights ahead to track approvals. This needs to be sufficiently large so that we can
/// recover after rather long network interruption, but not too large to consume too much memory if
/// someone in the network spams with invalid approvals. Note that we will only store approvals for
/// heights that are targeting us, which is once per as many heights as there are block producers,
/// thus 10_000 heights in practice will mean on the order of one hundred entries.
/// 승인을 추적하기 위해 몇 개의 높이를 앞두고 있습니다.
/// 이 값은 네트워크가 장시간 중단된 후에도 복구할 수 있을 만큼 충분히 커야 하지만,
/// 네트워크의 누군가가 잘못된 승인으로 스팸을 보내는 경우 메모리를 너무 많이 소모할 정도로 너무 크면 안 됩니다.
/// 저희를 대상으로 하는 높이의 승인만 저장하며,
/// 이는 블록 생성자 수만큼의 높이당 한 번씩이므로 실제로 10_000 높이는 100개 정도의 항목을 의미합니다.
const MAX_HEIGHTS_AHEAD_TO_STORE_APPROVALS: BlockHeight = 10_000;

// Number of blocks (before head) for which to keep the history of approvals (for debugging).
// (디버깅을 위해) 승인 내역을 보관할 블록 수(헤드 앞)입니다.
const MAX_HEIGHTS_BEFORE_TO_STORE_APPROVALS: u64 = 20;

// Maximum amount of historical approvals that we'd keep for debugging purposes.
// 디버깅 목적으로 보관할 수 있는 최대 승인 기록 양입니다.
const MAX_HISTORY_SIZE: usize = 1000;


/// The threshold for doomslug to create a block.
/// `TwoThirds` means the block can only be produced if at least 2/3 of the stake is approving it,
///             and is what should be used in production (and what guarantees finality)
/// `NoApprovals` means the block production is not blocked on approvals. This is used
///             in many tests (e.g. `cross_shard_tx`) to create lots of forkfulness.
/// 블록을 생성
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum DoomslugThresholdMode { //둠슬러그 임계값 모드
    NoApprovals,// 승인에 따라 블록 생성이 차단되지 않음을 의미.
    TwoThirds,  // 2/3 이상이 승인해야만 블록 생성 가능
}

/// The result of processing an approval.
/// 승인 처리 결과
#[derive(PartialEq, Eq, Debug)]
pub enum DoomslugBlockProductionReadiness {
    NotReady,
    ReadySince(Instant),
}
impl DoomslugBlockProductionReadiness{
    fn toStr(&self){

    }
}

struct DoomslugTimer {
    started: Instant,
    last_endorsement_sent: Instant,
    height: BlockHeight,
    endorsement_delay: Duration, // 블록 생성자에게 timer.height 로 승인을 전송하는 시간
    min_delay: Duration,
    delay_step: Duration,
    max_delay: Duration,
}

struct DoomslugTip {
    block_hash: CryptoHash,
    height: BlockHeight,
}

struct DoomslugApprovalsTracker {
    witness: HashMap<AccountId, (Approval, chrono::DateTime<chrono::Utc>)>,
    account_id_to_stakes: HashMap<AccountId, (Balance, Balance)>,
    total_stake_this_epoch: Balance,
    approved_stake_this_epoch: Balance,
    total_stake_next_epoch: Balance,
    approved_stake_next_epoch: Balance,
    time_passed_threshold: Option<Instant>,
    threshold_mode: DoomslugThresholdMode,
}

/// Approvals can arrive before the corresponding blocks, and we need a meaningful way to keep as
/// many approvals as possible that can be useful in the future, while not allowing an adversary
/// to spam us with invalid approvals.
/// To that extent, for each `account_id` and each `target_height` we keep exactly one approval,
/// whichever came last. We only maintain those for
///  a) `account_id`s that match the corresponding epoch (and for which we can validate a signature)
///  b) `target_height`s for which we produce blocks
///  c) `target_height`s within a meaningful horizon from the current tip.
/// This class is responsible for maintaining witnesses for the blocks, while also ensuring that
/// only one approval per (`account_id`) is kept. We instantiate one such class per height, thus
/// ensuring that only one approval is kept per (`target_height`, `account_id`). `Doomslug` below
/// ensures that only instances within the horizon are kept, and the user of the `Doomslug` is
/// responsible for ensuring that only approvals for proper account_ids with valid signatures are
/// provided.
/// 승인은 해당 블록보다 먼저 도착할 수 있으며,
/// 향후에 유용할 수 있는 승인은 최대한 많이 유지하면서 공격자가 스팸을 보내지 않도록 해야 합니다.
/// 이를 위해 각 '계정_id'와 각 '대상_높이'에 대해 마지막에 발생한 승인 중 정확히 하나의 승인만 보관합니다.
/// 다음 경우에만 유지합니다.
/// a) 해당 에포크와 일치하는 (그리고 서명을 검증할 수 있는) `account_id`.
/// b) 블록을 생성하는 `target_height`에 대해서만 유지합니다.
/// c) 현재 팁에서 의미 있는 지평선 내에 있는 `target_height`.
///
/// 이 클래스는 블록에 대한 증인을 유지 관리하는 동시에 (`account_id`) 당 하나의 승인만 유지되도록 합니다.
/// 높이당 이러한 클래스를 하나씩 인스턴스화하여
/// (`target_height`, `account_id`) 당 하나의 승인만 유지되도록 합니다.
/// 아래의 `Doomslug`는 수평선 내의 인스턴스만 유지되도록 하며,
/// `Doomslug`의 사용자는 유효한 서명이 있는 적절한 계정_id에 대한 승인만 제공되도록 할 책임이 있습니다.
struct DoomslugApprovalsTrackersAtHeight { //둠슬러그 승인 트래커의 높이
    approval_trackers: HashMap<ApprovalInner, DoomslugApprovalsTracker>,//승인 추적기
    last_approval_per_account: HashMap<AccountId, ApprovalInner>,//계정 당 마지막 승인
    // ApprovalInner:승인 및 스킵에 대해 다른 블록 승인 부분
}

/// Contains all the logic for Doomslug, but no integration with chain or storage. The integration
/// happens via `PersistentDoomslug` struct. The split is to simplify testing of the logic separate
/// from the chain.
/// 둠슬러그에 대한 모든 로직을 포함하지만 체인이나 스토리지와의 통합은 없습니다.
/// 통합은 `PersistentDoomslug` 구조체를 통해 이루어집니다.
/// 이 분할은 체인에서 분리된 로직의 테스트를 단순화하기 위한 것입니다.
pub struct Doomslug {
    approval_tracking: HashMap<BlockHeight, DoomslugApprovalsTrackersAtHeight>,
    /// Largest target height for which we issued an approval
    /// 승인을 발행한 최대 목표 높이
    largest_target_height: BlockHeight,

    /// Largest height for which we saw a block containing 1/2 endorsements in it
    /// 1/2 추천이 포함된 블록이 표시된 가장 큰 높이입니다.
    largest_final_height: BlockHeight,

    /// Largest height for which we saw threshold approvals (and thus can potentially create a block)
    /// 임계값 승인이 확인된 가장 큰 높이(따라서 잠재적으로 블록을 생성할 수 있음)
    largest_threshold_height: BlockHeight,

    /// Largest target height of approvals that we've received
    /// /// 지금까지 받은 승인 중 가장 큰 목표 높이
    largest_approval_height: BlockHeight,

    /// Information Doomslug tracks about the chain tip
    /// 체인 팁에 대해 둠슬러그가 추적하는 정보
    tip: DoomslugTip,

    /// Whether an endorsement (or in general an approval) was sent since updating the tip
    /// 팁을 업데이트한 후 승인(또는 일반적으로 승인)이 전송되었는지 여부
    endorsement_pending: bool, //#Consensus#messages

    /// Information to track the timer (see `start_timer` routine in the paper)
    /// 타이머를 추적하기 위한 정보(페이퍼의 `start_timer` 루틴 참조)
    timer: DoomslugTimer,

    signer: Option<Arc<dyn ValidatorSigner>>,
    /// How many approvals to have before producing a block. In production should be always `HalfStake`,
    ///    but for many tests we use `NoApprovals` to invoke more forkfulness
    /// 블록을 생성하기 전에 얼마나 많은 승인을 받아야 하는가.
    /// 프로덕션에서는 항상 `HalfStake`여야 합니다,
    /// 하지만 많은 테스트에서는 더 많은 포크풀을 호출하기 위해 `NoApprovals`를 사용합니다.
    threshold_mode: DoomslugThresholdMode,

    /// Approvals that were created by this doomslug instance (for debugging only).
    /// Keeps up to MAX_HISTORY_SIZE entries.
    /// 이 둠슬러그 인스턴스에 의해 생성된 승인(디버깅 전용).
    /// MAX_HISTORY_SIZE 항목까지 유지합니다.
    history: VecDeque<ApprovalHistoryEntry>,
}

impl DoomslugTimer {
    /// Computes the delay to sleep given the number of heights from the last final block
    /// This is what `T` represents in the paper.
    /// 마지막 최종 블록의 높이가 주어졌을 때 sleep 까지의 딜레이 계산
    /// # Arguments
    /// * `n` - number of heights since the last block with doomslug finality
    /// 둠슬러그가 최종 확정된 마지막 블록 이후의 높이 수
    /// # Returns
    /// Duration to sleep
    pub fn get_delay(&self, n: BlockHeightDelta) -> Duration { //메시지
        file_write("get_delay():마지막 최종 블록의 높이가 주어졌을 때 sleep 까지의 딜레이 계산").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("get_delay():마지막 최종 블록의 높이가 주어졌을 때 sleep 까지의 딜레이 계산");
        let n32 = u32::try_from(n).unwrap_or(u32::MAX);
        std::cmp::min(self.max_delay, self.min_delay + self.delay_step * n32.saturating_sub(2))
    }
}
//Approvals 추적기
impl DoomslugApprovalsTracker {
    fn new(
        account_id_to_stakes: HashMap<AccountId, (Balance, Balance)>,
        threshold_mode: DoomslugThresholdMode, //둠슬러그 임계값 모드 NoApprovals, TwoThirds
    ) -> Self {
        file_write("new() impl DoomslugApprovalsTracker").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("new() in impl DoomslugApprovalsTracker:Approvals 추적기 생성");
        let total_stake_this_epoch = account_id_to_stakes.values().map(|(x, _)| x).sum::<Balance>();
        let total_stake_next_epoch = account_id_to_stakes.values().map(|(_, x)| x).sum::<Balance>();

        DoomslugApprovalsTracker {
            witness: Default::default(), //HashMap<AccountId, (Approval, chrono::DateTime<chrono::Utc>)>,
            account_id_to_stakes, //:HashMap<AccountId, (Balance, Balance)>,
            total_stake_this_epoch,
            total_stake_next_epoch,
            approved_stake_this_epoch: 0, //:balance
            approved_stake_next_epoch: 0, //:balance
            time_passed_threshold: None, //timestamp
            threshold_mode,
        }
    }

    /// Given a single approval (either an endorsement or a skip-message) updates the approved
    /// stake on the block that is being approved, and returns whether the block is now ready to be
    /// produced.
    /// 단일 승인이 주어지면 승인 중인 블록의 승인된 지분을 업데이트하고 블록이 이제 생성될 준비가 되었는지 여부를 반환
    /// # Arguments
    /// * now      - the current timestamp
    /// * approval - the approval to process
    ///
    /// # Returns
    /// Whether the block is ready to be produced
    fn process_approval(
        &mut self,
        now: Instant,
        approval: &Approval,
    ) -> DoomslugBlockProductionReadiness {
        file_write("process_approval():주어진 단일 승인 중인 블록의 승인된 지분을 업데이트, 블록 생성 준비 여부 반환 impl DoomslugApprovalsTracker").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("process_approval() in impl DoomslugApprovalsTrackers");
        // DoomslugBlockProductionReadiness
        // enum{NotReady,ReadySince(Instant),}
        let mut increment_approved_stake = false;
        self.witness.entry(approval.account_id.clone()).or_insert_with(|| {
            increment_approved_stake = true;
            (approval.clone(), chrono::Utc::now())
        });

        if increment_approved_stake {
            let stakes = self.account_id_to_stakes.get(&approval.account_id).map_or((0, 0), |x| *x);
            self.approved_stake_this_epoch += stakes.0;
            self.approved_stake_next_epoch += stakes.1;
        }

        // We call to `get_block_production_readiness` here so that if the number of approvals crossed
        // the threshold, the timer for block production starts.
        // 승인 횟수가 임계값을 넘으면 블록 생성 타이머가 시작되도록 여기서
        // `get_block_production_readiness`를 호출합니다.
        self.get_block_production_readiness(now)
    }

    /// Withdraws an approval. This happens if a newer approval for the same `target_height` comes
    /// from the same account. Removes the approval from the `witness` and updates approved and
    /// endorsed stakes.
    /// 승인을 철회합니다.
    /// 동일한 계정에서 동일한 `target_height`에 대한 새로운 승인이 들어올 경우 발생합니다.
    /// witness 에서 승인을 제거하고 승인 및 승인된 지분을 업데이트합니다.
    fn withdraw_approval(&mut self, account_id: &AccountId) {
        file_write("withdraw_approval():승인 철회").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("withdraw_approval() impl DoomslugApprovalsTracker");
        let approval = match self.witness.remove(account_id) {
            None => return,
            Some(approval) => approval.0,
        };

        let stakes = self.account_id_to_stakes.get(&approval.account_id).map_or((0, 0), |x| *x);
        self.approved_stake_this_epoch -= stakes.0;
        self.approved_stake_next_epoch -= stakes.1;
    }

    /// Returns whether the block has enough approvals, and if yes, since what moment it does.
    ///
    /// # Arguments
    /// * now - the current timestamp
    ///
    /// # Returns
    /// `NotReady` if the block doesn't have enough approvals yet to cross the threshold
    /// `ReadySince` if the block has enough approvals to pass the threshold, and since when it
    ///     does
    /// 블록 생산 준비 상태 확인
    fn get_block_production_readiness(&mut self, now: Instant) -> DoomslugBlockProductionReadiness { //DBPR:NotReady,ReadySince(Instant),
        file_write("get_block_production_readiness():블록 생산 준비 상태 확인 impl DoomslugApprovalsTracker").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("get_block_production_readiness():블록 생산 준비 상태 확인 ");
        //approved_stake_this_epoch,total_stake_this_epoch :: balance
        if (self.approved_stake_this_epoch > self.total_stake_this_epoch * 2 / 3
            && (self.approved_stake_next_epoch > self.total_stake_next_epoch * 2 / 3
                || self.total_stake_next_epoch == 0))
            || self.threshold_mode == DoomslugThresholdMode::NoApprovals
        {
            if self.time_passed_threshold == None {
                self.time_passed_threshold = Some(now);
            }
            log("ReadySince");
            DoomslugBlockProductionReadiness::ReadySince(self.time_passed_threshold.unwrap())
        } else {
            log("NotReady");
            DoomslugBlockProductionReadiness::NotReady
        }
    }

    // Get witnesses together with their arrival time.
    // witnesses 의 도착 시간과 함께 witnesses 을 확보하세요.
    fn get_witnesses(&self) -> Vec<(AccountId, chrono::DateTime<chrono::Utc>)> {
        file_write("get_witnesses() impl DoomslugApprovalsTracker" ).unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("get_witnesses()");
        self.witness
            .iter()
            .map(|(key, (_, arrival_time))| (key.clone(), *arrival_time))
            .collect::<Vec<_>>()
    }
}

impl DoomslugApprovalsTrackersAtHeight {
    fn new() -> Self {
        file_write("new() impl DoomslugApprovalsTrackersAtHeight").unwrap();
        Self {
            approval_trackers: HashMap::new(), last_approval_per_account: HashMap::new()
        }
    }

    /// This method is a wrapper around `DoomslugApprovalsTracker::process_approval`, see comment
    /// above it for more details.
    /// This method has an extra logic that ensures that we only track one approval per `account_id`,
    /// if we already know some other approval for this account, we first withdraw it from the
    /// corresponding tracker, and associate the new approval with the account.
    ///
    /// # Arguments
    /// * `now`      - the current timestamp
    /// * `approval` - the approval to be processed
    /// * `stakes`   - all the stakes of all the block producers in the current epoch
    /// * `threshold_mode` - how many approvals are needed to produce a block. Is used to compute
    ///                the return value
    ///
    /// # Returns
    /// Same as `DoomslugApprovalsTracker::process_approval`
    ///
    /// 이 메서드에는 `account_id` 당 하나의 승인만 추적하도록 하는 추가 로직이 있는데,
    /// 이 계정에 대한 다른 승인을 이미 알고 있는 경우 먼저 해당 트래커에서 해당 승인을 철회한 다음
    /// 해당 트래커에서 철회하고 새 승인을 해당 계정과 연결합니다.
    ///
    fn process_approval(
        &mut self,
        now: Instant, // 현재 타임스탬프
        approval: &Approval, // 처리할 approval
        stakes: &[(ApprovalStake, bool)], //현재 에포크에 있는 모든 블록 생산자의 모든 지분.
        threshold_mode: DoomslugThresholdMode, //블록을 생성하는 데 필요한 승인 수입니다. 반환값을 계산하는 데 사용됩니다.
    ) -> DoomslugBlockProductionReadiness { //enum{NotReady,ReadySince(Instant),}
        //조건 binding last_parent: &ApprovalInner
        //계정 당 마지막 승인을 가져와서

        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("process_approval() in impl DoomslugApprovalsTrackersAtHeight");
        if let Some(last_parent) = self.last_approval_per_account.get(&approval.account_id) {
            let should_remove = self
                .approval_trackers
                .get_mut(last_parent)
                .map(|x| {
                    x.withdraw_approval(&approval.account_id);
                    x.witness.is_empty()
                })
                .unwrap_or(false);
            if should_remove {
                self.approval_trackers.remove(last_parent);
            }
        }

        let account_id_to_stakes = stakes
            .iter()
            .filter_map(|(x, is_slashed)| {
                if *is_slashed {
                    None
                } else {
                    Some((x.account_id.clone(), (x.stake_this_epoch, x.stake_next_epoch)))
                }
            })
            .collect::<HashMap<_, _>>();

        assert_eq!(account_id_to_stakes.len(), stakes.len());

        if !account_id_to_stakes.contains_key(&approval.account_id) {
            return DoomslugBlockProductionReadiness::NotReady;
        }

        self.last_approval_per_account.insert(approval.account_id.clone(), approval.inner.clone());
        self.approval_trackers
            .entry(approval.inner.clone())
            .or_insert_with(|| DoomslugApprovalsTracker::new(account_id_to_stakes, threshold_mode))
            .process_approval(now, approval)
    }

    /// Returns the current approvals status for the trackers at this height.
    /// Status contains information about which account voted (and for what) and whether the doomslug voting threshold was reached.
    /// 이 높이의 트래커에 대한 현재 승인 상태를 반환합니다.
    /// 상태에는 투표한 계정(및 대상)과 둠슬러그 투표 임계값에 도달했는지 여부에 대한 정보가 포함됩니다.
    pub fn status(&self) -> ApprovalAtHeightStatus {
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("status():이 높이의 트래커에 대한 현재 승인 상태를 반환합니다.");
        file_write("status():이 높이의 트래커에 대한 현재 승인 상태를 반환합니다 impl DoomslugApprovalsTrackersAtHeight").unwrap();
        let approvals = self
            .approval_trackers
            .iter()
            .flat_map(|(approval, tracker)| {
                let witnesses = tracker.get_witnesses();
                witnesses.into_iter().map(|(account_name, approval_time)| {
                    (account_name, (approval.clone(), approval_time))
                })
            })
            .collect::<HashMap<_, _>>();

        let threshold_approval = self
            .approval_trackers
            .iter()
            .filter_map(|(_, tracker)| tracker.time_passed_threshold)
            .min()
            .map(|ts| {
                chrono::Utc::now()
                    - chrono::Duration::from_std(ts.elapsed()).unwrap_or(chrono::Duration::days(1))
            });
        // Information about the approvals that we received.
        // 받은 승인에 대한 정보입니다.
        ApprovalAtHeightStatus { approvals, ready_at: threshold_approval }
    }
}

impl Doomslug {
    pub fn new(
        largest_target_height: BlockHeight,
        endorsement_delay: Duration,
        min_delay: Duration,
        delay_step: Duration,
        max_delay: Duration,
        signer: Option<Arc<dyn ValidatorSigner>>,
        threshold_mode: DoomslugThresholdMode,
    ) -> Self {
        file_write("new() impl doomslug").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("new() in impl Doomslug");
        Doomslug {
            approval_tracking: HashMap::new(),
            largest_target_height,
            largest_approval_height: 0, // 지금까지 받은 승인 중 가장 큰 목표 높이
            largest_final_height: 0, // 1/2 추천이 포함된 블록이 표시된 가장 큰 높이입니다.
            largest_threshold_height: 0, //임계값 승인이 확인된 가장 큰 높이(따라서 잠재적으로 블록을 생성할 수 있음)
            tip: DoomslugTip { block_hash: CryptoHash::default(), height: 0 },
            endorsement_pending: false,
            timer: DoomslugTimer {
                started: StaticClock::instant(),
                last_endorsement_sent: StaticClock::instant(),
                height: 0,
                endorsement_delay,
                min_delay,
                delay_step,
                max_delay,
            },
            signer,
            threshold_mode,
            history: VecDeque::new(),
        }
    }

    #[cfg(feature = "test_features")]
    pub fn adv_disable(&mut self) {
        file_write("adv_disable() impl Doomslug").unwrap();
        self.threshold_mode = DoomslugThresholdMode::NoApprovals
    }

    /// Returns the `(hash, height)` of the current tip. Currently is only used by tests.
    /// 현재 팁의 `(해시, 높이)`를 반환합니다. 현재는 테스트에서만 사용됩니다.
    pub fn get_tip(&self) -> (CryptoHash, BlockHeight) {
        file_write("get_tip() impl Doomslug").unwrap();
        (self.tip.block_hash, self.tip.height)
    }

    /// Returns the largest height for which we have enough approvals to be theoretically able to
    ///     produce a block (in practice a blocks might not be produceable yet if not enough time
    ///     passed since it accumulated enough approvals)
    /// 이론적으로 블록을 생성할 수 있을 만큼 충분한 승인을 받은 최대 높이를 반환합니다
    /// (실제로는 충분한 승인을 받은 후 충분한 시간이 지나지 않은 경우 블록을 생성할 수 없을 수 있습니다).
    pub fn get_largest_height_crossing_threshold(&self) -> BlockHeight {
        file_write("get_largest_height_crossing_threshold() impl Doomslug").unwrap();
        self.largest_threshold_height
    }

    /// Returns the largest height for which we've received an approval
    pub fn get_largest_approval_height(&self) -> BlockHeight {
        self.largest_approval_height
    }

    pub fn get_largest_final_height(&self) -> BlockHeight {
        self.largest_final_height
    }

    pub fn get_largest_target_height(&self) -> BlockHeight {
        self.largest_target_height
    }

    pub fn get_timer_height(&self) -> BlockHeight {
        self.timer.height
    }

    pub fn get_timer_start(&self) -> Instant {
        self.timer.started
    }

    /// Returns currently available approval history.
    /// 현재 사용 가능한 승인 내역 반환
    pub fn get_approval_history(&self) -> Vec<ApprovalHistoryEntry> {
        file_write("get_approval_history():현재 사용 가능한 승인 내역 반환 impl Doomslug").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("get_approval_history()");
        self.history.iter().cloned().collect::<Vec<_>>()
    }

    /// Adds new approval to the history.
    /// 내역에 새 승인을 추가합니다.
    fn update_history(&mut self, entry: ApprovalHistoryEntry) {
        file_write("update_history():내역에 새 승인을 추가합니다 impl Doomslug").unwrap();
        while self.history.len() >= MAX_HISTORY_SIZE {
            self.history.pop_front();
        }
        self.history.push_back(entry);
    }

    /// Is expected to be called periodically and processed the timer (`start_timer` in the paper)
    /// If the `cur_time` way ahead of last time the `process_timer` was called, will only process
    /// a bounded number of steps, to avoid an infinite loop in case of some bugs.
    /// Processes sending delayed approvals or skip messages
    /// A major difference with the paper is that we process endorsement from the `process_timer`,
    /// not at the time of receiving a block. It is done to stagger blocks if the network is way
    /// too fast (e.g. during tests, or if a large set of validators have connection significantly
    /// better between themselves than with the rest of the validators)
    ///
    /// 주기적으로 호출되어 타이머를 처리할 것으로 예상되는 경우(논문에서는 `start_timer`)
    /// `cur_time`이 `process_timer`를 마지막으로 호출한 시간보다 앞선 경우,
    /// 일부 버그 발생 시 무한 루프를 피하기 위해 제한된 수의 단계만 처리할 것입니다.
    /// 지연된 승인 또는 건너뛰기 메시지를 보내는 프로세스 블록을 수신하는 시점이 아니라
    /// `process_timer`에서 승인을 처리한다는 점이 논문과의 가장 큰 차이점입니다.
    /// 이는 네트워크가 너무 빠를 경우(예: 테스트 중이거나 대규모 검증자 세트가 나머지 검증자보다 훨씬 더 잘 연결되어 있는 경우)
    /// 블록을 엇갈리게 처리하기 위해 수행됩니다.
    ///
    /// # Arguments
    /// * `cur_time` - is expected to receive `now`. Doesn't directly use `now` to simplify testing
    ///
    /// # Returns
    /// A vector of approvals that need to be sent to other block producers as a result of processing
    /// the timers
    /// 타이머 처리의 결과로 다른 블록 생성자에게 보내야 하는 승인 벡터입니다.
    #[must_use]
    pub fn process_timer(&mut self, cur_time: Instant) -> Vec<Approval> { //#Consensus#messages(?) 메시지
        file_write("process_timer() impl Doomslug").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("process_timer()");
        println!("Approvals 담는 벡터 선언");
        let mut ret = vec![];
        for _ in 0..MAX_TIMER_ITERS {
            let skip_delay =
                self.timer.get_delay(self.timer.height.saturating_sub(self.largest_final_height));

            // The `endorsement_delay` is time to send approval to the block producer at `timer.height`,
            // while the `skip_delay` is the time before sending the approval to BP of `timer_height + 1`,
            // so it makes sense for them to be at least 2x apart
            debug_assert!(skip_delay >= 2 * self.timer.endorsement_delay);

            let tip_height = self.tip.height;

            if self.endorsement_pending
                && cur_time >= self.timer.last_endorsement_sent + self.timer.endorsement_delay
            {
                if tip_height >= self.largest_target_height {
                    self.largest_target_height = tip_height + 1;

                    if let Some(approval) = self.create_approval(tip_height + 1) {
                        ret.push(approval);
                    }
                    self.update_history(ApprovalHistoryEntry {
                        parent_height: tip_height,
                        target_height: tip_height + 1,
                        timer_started_ago_millis: self
                            .timer
                            .last_endorsement_sent
                            .elapsed()
                            .as_millis() as u64,
                        expected_delay_millis: self.timer.endorsement_delay.as_millis() as u64,
                        approval_creation_time: chrono::Utc::now(),
                    });
                }

                self.timer.last_endorsement_sent = cur_time;
                self.endorsement_pending = false;
            }

            if cur_time >= self.timer.started + skip_delay {
                debug_assert!(!self.endorsement_pending);

                self.largest_target_height =
                    std::cmp::max(self.timer.height + 1, self.largest_target_height);

                if let Some(approval) = self.create_approval(self.timer.height + 1) {
                    ret.push(approval);
                }
                self.update_history(ApprovalHistoryEntry {
                    parent_height: tip_height,
                    target_height: self.timer.height + 1,
                    timer_started_ago_millis: self.timer.started.elapsed().as_millis() as u64,
                    expected_delay_millis: skip_delay.as_millis() as u64,
                    approval_creation_time: chrono::Utc::now(),
                });

                // Restart the timer
                self.timer.started += skip_delay;
                self.timer.height += 1;
            } else {
                break;
            }
        }
        println!("Contents of the ret vector:{}",ret.len());
        for approval in &ret {
            println!("{:?}", approval);
        }
        ret
    }
    //1
    fn create_approval(&self, target_height: BlockHeight) -> Option<Approval> {
        file_write("create_approval() impl Doomslug").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("create_approval():승인 생성");
        self.signer.as_ref().map(|signer| {
            Approval::new(self.tip.block_hash, self.tip.height, target_height, &**signer)
        })
    }

    /// Determines whether a block has enough approvals to be produced.
    /// In production (with `mode == HalfStake`) we require the total stake of all the approvals to
    /// be strictly more than half of the total stake. For many non-doomslug specific tests
    /// (with `mode == NoApprovals`) no approvals are needed.
    ///
    /// # Arguments
    /// * `mode`      - whether we want half of the total stake or just a single approval
    /// * `approvals` - the set of approvals in the current block
    /// * `stakes`    - the vector of validator stakes in the current epoch
    pub fn can_approved_block_be_produced( //충분한 승인이 있는지 여부 결정
        mode: DoomslugThresholdMode,
        approvals: &[Option<Signature>],
        stakes: &[(Balance, Balance, bool)],
    ) -> bool {
        file_write("can_approved_block_be_produced:충분한 승인이 있는지 여부 결정 impl doomslug").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("can_approved_block_be_produced():충분한 승인이 있는지 여부 결정");
        if mode == DoomslugThresholdMode::NoApprovals {
            log("NoApprovals");
            return true;
        }

        let threshold1 = stakes.iter().map(|(x, _, _)| x).sum::<Balance>() * 2 / 3;
        let threshold2 = stakes.iter().map(|(_, x, _)| x).sum::<Balance>() * 2 / 3;
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log(threshold2.to_string().as_str());
        let approved_stake1 = approvals
            .iter()
            .zip(stakes.iter())
            .filter(|(_, (_, _, is_slashed))| !*is_slashed)
            .map(|(approval, (stake, _, _))| if approval.is_some() { *stake } else { 0 })
            .sum::<Balance>();

        let approved_stake2 = approvals
            .iter()
            .zip(stakes.iter())
            .filter(|(_, (_, _, is_slashed))| !*is_slashed)
            .map(|(approval, (_, stake, _))| if approval.is_some() { *stake } else { 0 })
            .sum::<Balance>();

        (approved_stake1 > threshold1 || threshold1 == 0)
            && (approved_stake2 > threshold2 || threshold2 == 0)
    }

    pub fn get_witness(
        &self,
        prev_hash: &CryptoHash,
        parent_height: BlockHeight,
        target_height: BlockHeight,
    ) -> HashMap<AccountId, (Approval, chrono::DateTime<chrono::Utc>)> {
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("get_witnesses() in impl doomslug");
        let hash_or_height = ApprovalInner::new(prev_hash, parent_height, target_height);
        if let Some(approval_trackers_at_height) = self.approval_tracking.get(&target_height) {
            let approvals_tracker =
                approval_trackers_at_height.approval_trackers.get(&hash_or_height);
            match approvals_tracker {
                None => HashMap::new(),
                Some(approvals_tracker) => approvals_tracker.witness.clone(),
            }
        } else {
            HashMap::new()
        }
    }

    /// Updates the current tip of the chain. Restarts the timer accordingly.
    /// 체인의 현재 tip 을 업데이트합니다. 그에 따라 타이머를 다시 시작합니다.
    /// # Arguments
    /// * `now`            - current time. Doesn't call to `Utc::now()` directly to simplify testing
    /// * `block_hash`     - the hash of the new tip
    /// * `height`         - the height of the tip
    /// * `last_ds_final_height` - last height at which a block in this chain has doomslug finality
    pub fn set_tip( //#Consensus#messages 메시지
        &mut self,
        now: Instant, //timestamp. 단순화하기 위해 Utc::now()를 직접 호출하지 않음.
        block_hash: CryptoHash, //새 팁의 해시
        height: BlockHeight, //팁의 높이
        last_final_height: BlockHeight, //체인의 블록이 둠슬러그 최종성을 갖는 마지막 높이
    ) {
        file_write("set_tip():체인의 현재 tip 을 업데이트 impl doomslug").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("set_tip()");
        debug_assert!(height > self.tip.height || self.tip.height == 0);
        self.tip = DoomslugTip { block_hash, height };

        self.largest_final_height = last_final_height;
        self.timer.height = height + 1;
        self.timer.started = now;

        self.approval_tracking.retain(|h, _| {
            *h > height.saturating_sub(MAX_HEIGHTS_BEFORE_TO_STORE_APPROVALS)
                && *h <= height + MAX_HEIGHTS_AHEAD_TO_STORE_APPROVALS
        });

        self.endorsement_pending = true;
    }

    /// Records an approval message, and return whether the block has passed the threshold / ready
    /// to be produced without waiting any further. See the comment for `DoomslugApprovalTracker::process_approval`
    /// for details
    /// 승인 메시지를 기록하고, 블록이 임계값을 통과했는지/더 이상 기다리지 않고 생성할 준비가 되었는지 반환합니다.
    #[must_use]
    fn on_approval_message_internal(
        &mut self,
        now: Instant,
        approval: &Approval,
        stakes: &[(ApprovalStake, bool)],
    ) -> DoomslugBlockProductionReadiness { //return :NotReady/ ReadySince
        file_write("on_approval_message_internal():승인 메시지 기록, 임계값 확인후 생성 준비 여부 반환 impl doomslug").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("on_approval_message_internal()");
        let threshold_mode = self.threshold_mode;
        let ret = self
            .approval_tracking
            .entry(approval.target_height)
            .or_insert_with(|| DoomslugApprovalsTrackersAtHeight::new())
            .process_approval(now, approval, stakes, threshold_mode);

        if approval.target_height > self.largest_approval_height {
            self.largest_approval_height = approval.target_height;
        }

        if ret != DoomslugBlockProductionReadiness::NotReady {
            if approval.target_height > self.largest_threshold_height {
                self.largest_threshold_height = approval.target_height;
            }
        }
        print!("DoomslugBlockProductionReadiness: {:?}",&ret);
        ret
    }

    /// Processes single approval
    /// 단일 승인 처리
    pub fn on_approval_message( //메시지
        &mut self,
        now: Instant,
        approval: &Approval,
        stakes: &[(ApprovalStake, bool)],
    ) {
        file_write("on_approval_message():단일 승인 처리 impl Doomslug").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("on_approval_message() : 단일 승인 처리");
        if approval.target_height < self.tip.height
            || approval.target_height > self.tip.height + MAX_HEIGHTS_AHEAD_TO_STORE_APPROVALS
        {
            return;
        }

        let _ = self.on_approval_message_internal(now, approval, stakes);
    }

    /// Gets the current status of approvals for a given height.
    /// It will only work for heights that we have in memory, that is that are not older than MAX_HEIGHTS_BEFORE_TO_STORE_APPROVALS
    /// blocks from the head.
    /// 주어진 높이에 대한 현재 승인 상태를 가져옵니다.
    /// 메모리에 있는 높이,
    /// 즉 머리에서 MAX_HEIGHTS_BEFORE_TO_STORE_APPROVALS 블록보다 오래되지 않은 높이에 대해서만 작동합니다.
    pub fn approval_status_at_height(&self, height: &BlockHeight) -> ApprovalAtHeightStatus {
        file_write("approval_status_at_height():주어진 높이에 대한 현재 승인 상태 get impl Doomslug").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("approval_status_at_height() :주어진 높이에 대한 현재 승인 상태를 가져옵니다.");
        self.approval_tracking.get(height).map(|it| it.status()).unwrap_or_default()
    }

    /// Returns whether we can produce a block for this height. The check for whether `me` is the
    /// block producer for the height needs to be done by the caller.
    /// We can produce a block if:
    ///  - The block has 2/3 of approvals, doomslug-finalizing the previous block, and we have
    ///    enough chunks, or
    ///  - The block has 1/2 of approvals, and T(h' / 6) has passed since the block has had 1/2 of
    ///    approvals for the first time, where h' is time since the last ds-final block.
    /// Only the height is passed into the function, we use the tip known to `Doomslug` as the
    /// parent hash.
    ///
    /// 이 높이의 블록 생성 가능 여부를 리턴
    /// 블록에 2/3 승인, 이전 블록을 doomslug-finalizing, 층븐힌 청크가 있거나
    /// 블록에 1/2 승인, 블록이 처음으로 1/2의 승인을 받은 이후 T(h'/6)가 지났으며, (h'= 마지막 ds-final 블록 이후 시간)
    /// 함수에 높이만 전달되며, 'Doomslug'로 알려진 팁을 부몬 해시로 사용.
    ///
    /// # Arguments:
    /// * `now`               - current timestamp
    /// * `target_height`     - the height for which the readiness is checked
    /// * `has_enough_chunks` - if not, we will wait for T(h' / 6) even if we have 2/3 approvals &
    ///                         have the previous block ds-final.
    #[must_use]
    pub fn ready_to_produce_block( //블록 생산할 준비 됐어?
        &mut self,
        now: Instant, //타임스탬프
        target_height: BlockHeight, //현재 블록 높이
        has_enough_chunks: bool, //충분한 청크 있는지 확인하는 변수
        log_block_production_info: bool, //
    ) -> bool {
        file_write("ready_to_produce_block:블록 생산할 준비 됐어? impl Doomslug").unwrap();
        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
        log("ready_to_produce_block():블록 생산할 준비 됐어?");
        let hash_or_height =
            ApprovalInner::new(&self.tip.block_hash, self.tip.height, target_height);
        if let Some(approval_trackers_at_height) = self.approval_tracking.get_mut(&target_height) {
            if let Some(approval_tracker) =
                approval_trackers_at_height.approval_trackers.get_mut(&hash_or_height)
            {
                let block_production_readiness =
                    approval_tracker.get_block_production_readiness(now);
                match block_production_readiness {
                    DoomslugBlockProductionReadiness::NotReady => false,
                    DoomslugBlockProductionReadiness::ReadySince(when) => {
                        if has_enough_chunks {
                            if log_block_production_info {
                                let message=format!("ready to produce block @ {}@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@", target_height);
                                file_write(message.as_str()).unwrap();
                                info!("ready to produce block @ {}, has enough approvals for {:?}, has enough chunks", target_height, now.saturating_duration_since(when));
                            }
                            true
                        } else {
                            let delay = self.timer.get_delay(
                                self.timer.height.saturating_sub(self.largest_final_height),
                            ) / 6;

                            let ready = now > when + delay;
                            if log_block_production_info {
                                if ready {
                                    info!("ready to produce block @ {}, has enough approvals for {:?}, does not have enough chunks", target_height, now.saturating_duration_since(when));
                                    let message=format!("ready to produce block @ {}@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@", target_height);
                                    file_write(message.as_str()).unwrap();
                                } else {
                                    info!("not ready to produce block @ {}, need to wait {:?}, has enough approvals for {:?}", target_height, (when + delay).saturating_duration_since(now), now.saturating_duration_since(when));
                                    /*let message=format!("not ready to produce block @ {}@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@", target_height);
                                    file_write(message.as_str()).unwrap();*/
                                }
                            }
                            ready
                        }
                    }
                }
            } else {
                false
            }
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use near_crypto::{KeyType, SecretKey};
    use near_primitives::block::{Approval, ApprovalInner};
    use near_primitives::hash::hash;
    use near_primitives::static_clock::StaticClock;
    use near_primitives::test_utils::create_test_signer;
    use near_primitives::types::ApprovalStake;

    use crate::doomslug::{
        DoomslugApprovalsTrackersAtHeight, DoomslugBlockProductionReadiness, DoomslugThresholdMode,
    };
    use crate::Doomslug;

    #[test]
    fn test_endorsements_and_skips_basic() {
        let mut ds = Doomslug::new(
            0,
            Duration::from_millis(400),
            Duration::from_millis(1000),
            Duration::from_millis(100),
            Duration::from_millis(3000),
            Some(Arc::new(create_test_signer("test"))),
            DoomslugThresholdMode::TwoThirds,
        );

        let mut now = StaticClock::instant(); // For the test purposes the absolute value of the initial instant doesn't matter

        // Set a new tip, must produce an endorsement
        ds.set_tip(now, hash(&[1]), 1, 1);
        assert_eq!(ds.process_timer(now + Duration::from_millis(399)).len(), 0);
        let approval =
            ds.process_timer(now + Duration::from_millis(400)).into_iter().nth(0).unwrap();
        assert_eq!(approval.inner, ApprovalInner::Endorsement(hash(&[1])));
        assert_eq!(approval.target_height, 2);

        // Same tip => no approval
        assert_eq!(ds.process_timer(now + Duration::from_millis(400)), vec![]);

        // The block was `ds_final` and therefore started the timer. Try checking before one second expires
        assert_eq!(ds.process_timer(now + Duration::from_millis(999)), vec![]);

        // But one second should trigger the skip
        match ds.process_timer(now + Duration::from_millis(1000)) {
            approvals if approvals.len() == 0 => assert!(false),
            approvals => {
                assert_eq!(approvals[0].inner, ApprovalInner::Skip(1));
                assert_eq!(approvals[0].target_height, 3);
            }
        }

        // Shift now 1 second forward
        now += Duration::from_millis(1000);

        // Not processing a block at height 2 should not produce an appoval
        ds.set_tip(now, hash(&[2]), 2, 0);
        assert_eq!(ds.process_timer(now + Duration::from_millis(400)), vec![]);

        // Shift now 1 second forward
        now += Duration::from_millis(1000);

        // But at height 3 should (also neither block has finality set, keep last final at 0 for now)
        ds.set_tip(now, hash(&[3]), 3, 0);
        let approval =
            ds.process_timer(now + Duration::from_millis(400)).into_iter().nth(0).unwrap();
        assert_eq!(approval.inner, ApprovalInner::Endorsement(hash(&[3])));
        assert_eq!(approval.target_height, 4);

        // Move 1 second further
        now += Duration::from_millis(1000);

        assert_eq!(ds.process_timer(now + Duration::from_millis(199)), vec![]);

        match ds.process_timer(now + Duration::from_millis(200)) {
            approvals if approvals.len() == 0 => assert!(false),
            approvals if approvals.len() == 1 => {
                assert_eq!(approvals[0].inner, ApprovalInner::Skip(3));
                assert_eq!(approvals[0].target_height, 5);
            }
            _ => assert!(false),
        }

        // Move 1 second further
        now += Duration::from_millis(1000);

        // Now skip 5 (the extra delay is 200+300 = 500)
        assert_eq!(ds.process_timer(now + Duration::from_millis(499)), vec![]);

        match ds.process_timer(now + Duration::from_millis(500)) {
            approvals if approvals.len() == 0 => assert!(false),
            approvals => {
                assert_eq!(approvals[0].inner, ApprovalInner::Skip(3));
                assert_eq!(approvals[0].target_height, 6);
            }
        }

        // Move 1 second further
        now += Duration::from_millis(1000);

        // Skip 6 (the extra delay is 0+200+300+400 = 900)
        assert_eq!(ds.process_timer(now + Duration::from_millis(899)), vec![]);

        match ds.process_timer(now + Duration::from_millis(900)) {
            approvals if approvals.len() == 0 => assert!(false),
            approvals => {
                assert_eq!(approvals[0].inner, ApprovalInner::Skip(3));
                assert_eq!(approvals[0].target_height, 7);
            }
        }

        // Move 1 second further
        now += Duration::from_millis(1000);

        // Accept block at 5 with finality on the prev block, expect it to not produce an approval
        ds.set_tip(now, hash(&[5]), 5, 4);
        assert_eq!(ds.process_timer(now + Duration::from_millis(400)), vec![]);

        // Skip a whole bunch of heights by moving 100 seconds ahead
        now += Duration::from_millis(100_000);
        assert!(ds.process_timer(now).len() > 10);

        // Add some random small number of milliseconds to test that when the next block is added, the
        // timer is reset
        now += Duration::from_millis(17);

        // No approval, since we skipped 6
        ds.set_tip(now, hash(&[6]), 6, 4);
        assert_eq!(ds.process_timer(now + Duration::from_millis(400)), vec![]);

        // The block height was less than the timer height, and thus the timer was reset.
        // The wait time for height 7 with last ds final block at 5 is 1100
        assert_eq!(ds.process_timer(now + Duration::from_millis(1099)), vec![]);

        match ds.process_timer(now + Duration::from_millis(1100)) {
            approvals if approvals.len() == 0 => assert!(false),
            approvals => {
                assert_eq!(approvals[0].inner, ApprovalInner::Skip(6));
                assert_eq!(approvals[0].target_height, 8);
            }
        }
    }

    #[test]
    fn test_doomslug_approvals() {
        let accounts: Vec<(&str, u128, u128)> =
            vec![("test1", 2, 0), ("test2", 1, 0), ("test3", 3, 0), ("test4", 1, 0)];
        let stakes = accounts
            .iter()
            .map(|(account_id, stake_this_epoch, stake_next_epoch)| ApprovalStake {
                account_id: account_id.parse().unwrap(),
                stake_this_epoch: *stake_this_epoch,
                stake_next_epoch: *stake_next_epoch,
                public_key: SecretKey::from_seed(KeyType::ED25519, account_id).public_key(),
            })
            .map(|stake| (stake, false))
            .collect::<Vec<_>>();
        let signers = accounts
            .iter()
            .map(|(account_id, _, _)| create_test_signer(account_id))
            .collect::<Vec<_>>();

        let signer = Arc::new(create_test_signer("test"));
        let mut ds = Doomslug::new(
            0,
            Duration::from_millis(400),
            Duration::from_millis(1000),
            Duration::from_millis(100),
            Duration::from_millis(3000),
            Some(signer),
            DoomslugThresholdMode::TwoThirds,
        );

        let mut now = StaticClock::instant();

        // In the comments below the format is
        // account, height -> approved stake
        // The total stake is 7, so the threshold is 5

        // "test1", 2 -> 2
        assert_eq!(
            ds.on_approval_message_internal(
                now,
                &Approval::new(hash(&[1]), 1, 2, &signers[0]),
                &stakes,
            ),
            DoomslugBlockProductionReadiness::NotReady,
        );

        // "test3", 4 -> 3
        assert_eq!(
            ds.on_approval_message_internal(
                now,
                &Approval::new(hash(&[1]), 1, 4, &signers[2]),
                &stakes,
            ),
            DoomslugBlockProductionReadiness::NotReady,
        );

        // "test4", 4 -> 4
        assert_eq!(
            ds.on_approval_message_internal(
                now,
                &Approval::new(hash(&[1]), 1, 4, &signers[3]),
                &stakes,
            ),
            DoomslugBlockProductionReadiness::NotReady,
        );

        // "test1", 4 -> same account, still 5
        assert_eq!(
            ds.on_approval_message_internal(
                now + Duration::from_millis(100),
                &Approval::new(hash(&[1]), 1, 4, &signers[3]),
                &stakes,
            ),
            DoomslugBlockProductionReadiness::NotReady,
        );

        // "test2", 4 -> 5
        assert_eq!(
            ds.on_approval_message_internal(
                now,
                &Approval::new(hash(&[1]), 1, 4, &signers[1]),
                &stakes,
            ),
            DoomslugBlockProductionReadiness::ReadySince(now),
        );

        // "test1", 4 -> 7
        assert_eq!(
            ds.on_approval_message_internal(
                now,
                &Approval::new(hash(&[1]), 1, 4, &signers[0]),
                &stakes,
            ),
            DoomslugBlockProductionReadiness::ReadySince(now),
        );

        // "test4", 2 -> 3
        assert_eq!(
            ds.on_approval_message_internal(
                now,
                &Approval::new(hash(&[1]), 1, 2, &signers[3]),
                &stakes,
            ),
            DoomslugBlockProductionReadiness::NotReady,
        );

        now += Duration::from_millis(200);

        // "test3", 2 -> 6
        assert_eq!(
            ds.on_approval_message_internal(
                now,
                &Approval::new(hash(&[1]), 1, 2, &signers[2]),
                &stakes,
            ),
            DoomslugBlockProductionReadiness::ReadySince(now),
        );

        // A different parent hash
        assert_eq!(
            ds.on_approval_message_internal(
                now,
                &Approval::new(hash(&[2]), 2, 4, &signers[1]),
                &stakes,
            ),
            DoomslugBlockProductionReadiness::NotReady,
        );
    }

    #[test]
    fn test_doomslug_one_approval_per_target_height() {
        let accounts = vec![("test1", 2, 0), ("test2", 1, 2), ("test3", 3, 3), ("test4", 2, 2)];
        let signers = accounts
            .iter()
            .map(|(account_id, _, _)| create_test_signer(account_id))
            .collect::<Vec<_>>();
        let stakes = accounts
            .into_iter()
            .map(|(account_id, stake_this_epoch, stake_next_epoch)| ApprovalStake {
                account_id: account_id.parse().unwrap(),
                stake_this_epoch,
                stake_next_epoch,
                public_key: SecretKey::from_seed(KeyType::ED25519, account_id).public_key(),
            })
            .map(|stake| (stake, false))
            .collect::<Vec<_>>();
        let mut tracker = DoomslugApprovalsTrackersAtHeight::new();

        let a1_1 = Approval::new(hash(&[1]), 1, 4, &signers[0]);
        let a1_2 = Approval::new(hash(&[1]), 1, 4, &signers[1]);
        let a1_3 = Approval::new(hash(&[1]), 1, 4, &signers[2]);

        let a2_1 = Approval::new(hash(&[3]), 3, 4, &signers[0]);
        let a2_2 = Approval::new(hash(&[3]), 3, 4, &signers[1]);
        let a2_3 = Approval::new(hash(&[3]), 3, 4, &signers[2]);

        // Process first approval, and then process it again and make sure it works
        tracker.process_approval(
            StaticClock::instant(),
            &a1_1,
            &stakes,
            DoomslugThresholdMode::TwoThirds,
        );

        assert_eq!(
            tracker
                .approval_trackers
                .get(&ApprovalInner::Skip(1))
                .unwrap()
                .approved_stake_this_epoch,
            2
        );

        assert_eq!(
            tracker
                .approval_trackers
                .get(&ApprovalInner::Skip(1))
                .unwrap()
                .approved_stake_next_epoch,
            0
        );

        tracker.process_approval(
            StaticClock::instant(),
            &a1_1,
            &stakes,
            DoomslugThresholdMode::TwoThirds,
        );

        assert_eq!(
            tracker
                .approval_trackers
                .get(&ApprovalInner::Skip(1))
                .unwrap()
                .approved_stake_this_epoch,
            2
        );

        assert_eq!(
            tracker
                .approval_trackers
                .get(&ApprovalInner::Skip(1))
                .unwrap()
                .approved_stake_next_epoch,
            0
        );

        // Process the remaining two approvals on the first block
        tracker.process_approval(
            StaticClock::instant(),
            &a1_2,
            &stakes,
            DoomslugThresholdMode::TwoThirds,
        );
        tracker.process_approval(
            StaticClock::instant(),
            &a1_3,
            &stakes,
            DoomslugThresholdMode::TwoThirds,
        );

        assert_eq!(
            tracker
                .approval_trackers
                .get(&ApprovalInner::Skip(1))
                .unwrap()
                .approved_stake_this_epoch,
            6
        );

        assert_eq!(
            tracker
                .approval_trackers
                .get(&ApprovalInner::Skip(1))
                .unwrap()
                .approved_stake_next_epoch,
            5
        );

        // Process new approvals one by one, expect the approved and endorsed stake to slowly decrease
        tracker.process_approval(
            StaticClock::instant(),
            &a2_1,
            &stakes,
            DoomslugThresholdMode::TwoThirds,
        );

        assert_eq!(
            tracker
                .approval_trackers
                .get(&ApprovalInner::Skip(1))
                .unwrap()
                .approved_stake_this_epoch,
            4
        );

        assert_eq!(
            tracker
                .approval_trackers
                .get(&ApprovalInner::Skip(1))
                .unwrap()
                .approved_stake_next_epoch,
            5
        );

        tracker.process_approval(
            StaticClock::instant(),
            &a2_2,
            &stakes,
            DoomslugThresholdMode::TwoThirds,
        );

        assert_eq!(
            tracker
                .approval_trackers
                .get(&ApprovalInner::Skip(1))
                .unwrap()
                .approved_stake_this_epoch,
            3
        );

        assert_eq!(
            tracker
                .approval_trackers
                .get(&ApprovalInner::Skip(1))
                .unwrap()
                .approved_stake_next_epoch,
            3
        );

        // As we update the last of the three approvals, the tracker for the first block should be completely removed
        tracker.process_approval(
            StaticClock::instant(),
            &a2_3,
            &stakes,
            DoomslugThresholdMode::TwoThirds,
        );

        assert!(tracker.approval_trackers.get(&ApprovalInner::Skip(1)).is_none());

        // Check the approved and endorsed stake for the new block, and also ensure that processing one of the same approvals
        // again works fine

        assert_eq!(
            tracker
                .approval_trackers
                .get(&ApprovalInner::Endorsement(hash(&[3])))
                .unwrap()
                .approved_stake_this_epoch,
            6
        );

        assert_eq!(
            tracker
                .approval_trackers
                .get(&ApprovalInner::Endorsement(hash(&[3])))
                .unwrap()
                .approved_stake_next_epoch,
            5
        );

        tracker.process_approval(
            StaticClock::instant(),
            &a2_3,
            &stakes,
            DoomslugThresholdMode::TwoThirds,
        );

        assert_eq!(
            tracker
                .approval_trackers
                .get(&ApprovalInner::Endorsement(hash(&[3])))
                .unwrap()
                .approved_stake_this_epoch,
            6
        );

        assert_eq!(
            tracker
                .approval_trackers
                .get(&ApprovalInner::Endorsement(hash(&[3])))
                .unwrap()
                .approved_stake_next_epoch,
            5
        );
    }
}
