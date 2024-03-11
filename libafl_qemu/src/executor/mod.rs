//! A `QEMU`-based executor for binary-only instrumentation in `LibAFL`
#[cfg(emulation_mode = "usermode")]
use core::ptr;
use core::{
    ffi::c_void,
    fmt::{self, Debug, Formatter},
    time::Duration,
};

#[cfg(feature = "fork")]
use libafl::{
    events::EventManager,
    executors::InProcessForkExecutor,
    state::{HasLastReportTime, HasMetadata},
};
use libafl::{
    events::{EventFirer, EventRestarter},
    executors::{
        hooks::inprocess::InProcessExecutorHandlerData,
        inprocess::{HasInProcessHooks, InProcessExecutor},
        Executor, ExitKind, HasObservers,
    },
    feedbacks::Feedback,
    fuzzer::HasObjective,
    observers::{ObserversTuple, UsesObservers},
    state::{HasCorpus, HasExecutions, HasSolutions, State, UsesState},
    Error,
};
use libafl_bolts::os::unix_signals::{siginfo_t, ucontext_t, Signal};
#[cfg(feature = "fork")]
use libafl_bolts::shmem::ShMemProvider;

use crate::{emu::Emulator, helper::QemuHelperTuple, hooks::QemuHooks, IsEmuExitHandler};

/// A version of `QemuExecutor` with a state accessible from the harness.
pub mod stateful;

pub struct QemuExecutorState<'a, QT, S, E>
where
    QT: QemuHelperTuple<S, E>,
    S: State + HasExecutions,
    E: IsEmuExitHandler,
{
    hooks: &'a mut QemuHooks<QT, S, E>,
    first_exec: bool,
}

pub struct QemuExecutor<'a, H, OT, QT, S, E>
where
    H: FnMut(&S::Input) -> ExitKind,
    S: State + HasExecutions,
    OT: ObserversTuple<S>,
    QT: QemuHelperTuple<S, E>,
    E: IsEmuExitHandler,
{
    inner: InProcessExecutor<'a, H, OT, S>,
    state: QemuExecutorState<'a, QT, S, E>,
}

impl<'a, H, OT, QT, S, E> Debug for QemuExecutor<'a, H, OT, QT, S, E>
where
    H: FnMut(&S::Input) -> ExitKind,
    S: State + HasExecutions,
    OT: ObserversTuple<S> + Debug,
    QT: QemuHelperTuple<S, E> + Debug,
    E: IsEmuExitHandler,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("QemuExecutor")
            .field("hooks", &self.state.hooks)
            .field("inner", &self.inner)
            .finish()
    }
}

#[cfg(emulation_mode = "usermode")]
extern "C" {
    // Original QEMU user signal handler
    fn libafl_qemu_handle_crash(signal: i32, info: *mut siginfo_t, puc: *mut c_void);
}

#[cfg(emulation_mode = "usermode")]
pub unsafe fn inproc_qemu_crash_handler<'a, E, EM, OF, Z, QT, S, EH>(
    signal: Signal,
    info: &'a mut siginfo_t,
    mut context: Option<&'a mut ucontext_t>,
    _data: &'a mut InProcessExecutorHandlerData,
) where
    E: Executor<EM, Z> + HasObservers,
    EM: EventFirer<State = E::State> + EventRestarter<State = E::State>,
    OF: Feedback<E::State>,
    E::State: HasExecutions + HasSolutions + HasCorpus,
    Z: HasObjective<Objective = OF, State = E::State>,
    QT: QemuHelperTuple<S, EH> + Debug + 'a,
    S: State + HasExecutions + 'a,
    EH: IsEmuExitHandler,
{
    let puc = match &mut context {
        Some(v) => ptr::from_mut::<ucontext_t>(*v) as *mut c_void,
        None => ptr::null_mut(),
    };
    libafl_qemu_handle_crash(signal as i32, info, puc);
}

#[cfg(emulation_mode = "systemmode")]
pub(crate) static mut BREAK_ON_TMOUT: bool = false;

#[cfg(emulation_mode = "systemmode")]
extern "C" {
    fn qemu_system_debug_request();
}

#[cfg(emulation_mode = "systemmode")]
pub unsafe fn inproc_qemu_timeout_handler<'a, E, EM, OF, Z>(
    signal: Signal,
    info: &'a mut siginfo_t,
    context: Option<&'a mut ucontext_t>,
    data: &'a mut InProcessExecutorHandlerData,
) where
    E: Executor<EM, Z> + HasObservers + HasInProcessHooks,
    EM: EventFirer<State = E::State> + EventRestarter<State = E::State>,
    OF: Feedback<E::State>,
    E::State: HasSolutions + HasCorpus + HasExecutions,
    Z: HasObjective<Objective = OF, State = E::State>,
{
    if BREAK_ON_TMOUT {
        qemu_system_debug_request();
    } else {
        libafl::executors::hooks::unix::unix_signal_handler::inproc_timeout_handler::<E, EM, OF, Z>(
            signal, info, context, data,
        );
    }
}

impl<'a, QT, S, EH> QemuExecutorState<'a, QT, S, EH>
where
    S: State + HasExecutions,
    QT: QemuHelperTuple<S, EH> + Debug,
    EH: IsEmuExitHandler,
{
    pub fn new<E, EM, OF, OT, Z>(hooks: &'a mut QemuHooks<QT, S, EH>) -> Result<Self, Error>
    where
        E: Executor<EM, Z, State = S> + HasInProcessHooks + HasObservers,
        EM: EventFirer<State = S> + EventRestarter<State = S>,
        OF: Feedback<S>,
        OT: ObserversTuple<S>,
        S: State + HasExecutions + HasCorpus + HasSolutions,
        Z: HasObjective<Objective = OF, State = S>,
    {
        #[cfg(emulation_mode = "usermode")]
        {
            let handler = |hooks: &mut QemuHooks<QT, S, EH>, host_sig| {
                eprintln!("Crashed with signal {host_sig}");
                unsafe {
                    libafl::executors::inprocess::generic_inproc_crash_handler::<E, EM, OF, Z>();
                }
                if let Some(cpu) = hooks.emulator().current_cpu() {
                    eprint!("Context:\n{}", cpu.display_context());
                }
            };

            hooks.crash_closure(Box::new(handler));
        }
        Ok(QemuExecutorState {
            first_exec: true,
            hooks,
        })
    }

    #[must_use]
    pub fn hooks(&self) -> &QemuHooks<QT, S, EH> {
        self.hooks
    }

    pub fn hooks_mut(&mut self) -> &mut QemuHooks<QT, S, EH> {
        self.hooks
    }

    #[must_use]
    pub fn emulator(&self) -> &Emulator<EH> {
        self.hooks.emulator()
    }
}

impl<'a, H, OT, QT, S, E> QemuExecutor<'a, H, OT, QT, S, E>
where
    H: FnMut(&S::Input) -> ExitKind,
    S: State + HasExecutions,
    OT: ObserversTuple<S>,
    QT: QemuHelperTuple<S, E> + Debug,
    E: IsEmuExitHandler,
{
    pub fn new<EM, OF, Z>(
        hooks: &'a mut QemuHooks<QT, S, E>,
        harness_fn: &'a mut H,
        observers: OT,
        fuzzer: &mut Z,
        state: &mut S,
        event_mgr: &mut EM,
        timeout: Duration,
    ) -> Result<Self, Error>
    where
        EM: EventFirer<State = S> + EventRestarter<State = S>,
        OF: Feedback<S>,
        S: State + HasExecutions + HasCorpus + HasSolutions,
        Z: HasObjective<Objective = OF, State = S>,
    {
        let mut inner = InProcessExecutor::with_timeout(
            harness_fn, observers, fuzzer, state, event_mgr, timeout,
        )?;

        #[cfg(emulation_mode = "usermode")]
        {
            inner.inprocess_hooks_mut().crash_handler =
                inproc_qemu_crash_handler::<InProcessExecutor<'a, H, OT, S>, EM, OF, Z, QT, S, E>
                    as *const c_void;
        }

        #[cfg(emulation_mode = "systemmode")]
        {
            inner.inprocess_hooks_mut().timeout_handler =
                inproc_qemu_timeout_handler::<InProcessExecutor<'a, H, OT, S>, EM, OF, Z>
                    as *const c_void;
        }

        let state =
            QemuExecutorState::new::<InProcessExecutor<'a, H, OT, S>, EM, OF, OT, Z>(hooks)?;

        Ok(Self { inner, state })
    }

    pub fn inner(&self) -> &InProcessExecutor<'a, H, OT, S> {
        &self.inner
    }

    #[cfg(emulation_mode = "systemmode")]
    pub fn break_on_timeout(&mut self) {
        unsafe {
            BREAK_ON_TMOUT = true;
        }
    }

    pub fn inner_mut(&mut self) -> &mut InProcessExecutor<'a, H, OT, S> {
        &mut self.inner
    }

    pub fn hooks(&self) -> &QemuHooks<QT, S, E> {
        self.state.hooks()
    }

    pub fn hooks_mut(&mut self) -> &mut QemuHooks<QT, S, E> {
        self.state.hooks_mut()
    }

    pub fn emulator(&self) -> &Emulator<E> {
        self.state.emulator()
    }
}

impl<'a, QT, S, EH> QemuExecutorState<'a, QT, S, EH>
where
    S: State + HasExecutions + HasCorpus + HasSolutions,
    QT: QemuHelperTuple<S, EH> + Debug,
    EH: IsEmuExitHandler,
{
    fn pre_exec<E, EM, OF, Z>(&mut self, input: &E::Input)
    where
        E: Executor<EM, Z, State = S>,
        EM: EventFirer<State = S> + EventRestarter<State = S>,
        OF: Feedback<S>,
        Z: HasObjective<Objective = OF, State = S>,
    {
        let emu = self.hooks.emulator().clone();
        if self.first_exec {
            self.hooks.helpers().first_exec_all(self.hooks);
            self.first_exec = false;
        }
        self.hooks.helpers_mut().pre_exec_all(&emu, input);
    }

    fn post_exec<E, EM, OT, OF, Z>(
        &mut self,
        input: &E::Input,
        observers: &mut OT,
        exit_kind: &mut ExitKind,
    ) where
        E: Executor<EM, Z, State = S> + HasObservers,
        EM: EventFirer<State = S> + EventRestarter<State = S>,
        OT: ObserversTuple<S>,
        OF: Feedback<S>,
        Z: HasObjective<Objective = OF, State = S>,
    {
        let emu = self.hooks.emulator().clone();
        self.hooks
            .helpers_mut()
            .post_exec_all(&emu, input, observers, exit_kind);
    }
}

impl<'a, EM, H, OT, OF, QT, S, Z, E> Executor<EM, Z> for QemuExecutor<'a, H, OT, QT, S, E>
where
    EM: EventFirer<State = S> + EventRestarter<State = S>,
    H: FnMut(&S::Input) -> ExitKind,
    S: State + HasExecutions + HasCorpus + HasSolutions,
    OT: ObserversTuple<S>,
    OF: Feedback<S>,
    QT: QemuHelperTuple<S, E> + Debug,
    Z: HasObjective<Objective = OF, State = S>,
    E: IsEmuExitHandler,
{
    fn run_target(
        &mut self,
        fuzzer: &mut Z,
        state: &mut Self::State,
        mgr: &mut EM,
        input: &Self::Input,
    ) -> Result<ExitKind, Error> {
        self.state.pre_exec::<Self, EM, OF, Z>(input);
        let mut exit_kind = self.inner.run_target(fuzzer, state, mgr, input)?;
        self.state.post_exec::<Self, EM, OT, OF, Z>(
            input,
            self.inner.observers_mut(),
            &mut exit_kind,
        );
        Ok(exit_kind)
    }
}

impl<'a, H, OT, QT, S, E> UsesState for QemuExecutor<'a, H, OT, QT, S, E>
where
    H: FnMut(&S::Input) -> ExitKind,
    OT: ObserversTuple<S>,
    QT: QemuHelperTuple<S, E>,
    S: State + HasExecutions,
    E: IsEmuExitHandler,
{
    type State = S;
}

impl<'a, H, OT, QT, S, E> UsesObservers for QemuExecutor<'a, H, OT, QT, S, E>
where
    H: FnMut(&S::Input) -> ExitKind,
    OT: ObserversTuple<S>,
    QT: QemuHelperTuple<S, E>,
    S: State + HasExecutions,
    E: IsEmuExitHandler,
{
    type Observers = OT;
}

impl<'a, H, OT, QT, S, E> HasObservers for QemuExecutor<'a, H, OT, QT, S, E>
where
    H: FnMut(&S::Input) -> ExitKind,
    S: State + HasExecutions,
    OT: ObserversTuple<S>,
    QT: QemuHelperTuple<S, E>,
    E: IsEmuExitHandler,
{
    #[inline]
    fn observers(&self) -> &OT {
        self.inner.observers()
    }

    #[inline]
    fn observers_mut(&mut self) -> &mut OT {
        self.inner.observers_mut()
    }
}

#[cfg(feature = "fork")]
pub struct QemuForkExecutor<'a, H, OT, QT, S, SP, E, EM, Z>
where
    H: FnMut(&S::Input) -> ExitKind,
    S: State + HasExecutions,
    OT: ObserversTuple<S>,
    QT: QemuHelperTuple<S, E>,
    SP: ShMemProvider,
    EM: UsesState<State = S>,
    Z: UsesState<State = S>,
    E: IsEmuExitHandler,
{
    inner: InProcessForkExecutor<'a, H, OT, S, SP, EM, Z>,
    state: QemuExecutorState<'a, QT, S, E>,
}

#[cfg(feature = "fork")]
impl<'a, H, OT, QT, S, SP, E, EM, Z> Debug for QemuForkExecutor<'a, H, OT, QT, S, SP, E, EM, Z>
where
    H: FnMut(&S::Input) -> ExitKind,
    S: State + HasExecutions,
    OT: ObserversTuple<S> + Debug,
    QT: QemuHelperTuple<S, E> + Debug,
    SP: ShMemProvider,
    E: IsEmuExitHandler,
    EM: UsesState<State = S>,
    Z: UsesState<State = S>,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("QemuForkExecutor")
            .field("hooks", &self.state.hooks)
            .field("inner", &self.inner)
            .finish()
    }
}

#[cfg(feature = "fork")]
impl<'a, H, OT, QT, S, SP, E, EM, Z, OF> QemuForkExecutor<'a, H, OT, QT, S, SP, E, EM, Z>
where
    H: FnMut(&S::Input) -> ExitKind,
    S: State + HasExecutions,
    OT: ObserversTuple<S>,
    QT: QemuHelperTuple<S, E>,
    SP: ShMemProvider,
    E: IsEmuExitHandler,
    EM: EventFirer<State = S> + EventRestarter,
    OF: Feedback<S>,
    S: HasSolutions,
    Z: HasObjective<Objective = OF, State = S>,
{
    pub fn new(
        hooks: &'a mut QemuHooks<QT, S, E>,
        harness_fn: &'a mut H,
        observers: OT,
        fuzzer: &mut Z,
        state: &mut S,
        event_mgr: &mut EM,
        shmem_provider: SP,
        timeout: core::time::Duration,
    ) -> Result<Self, Error> {
        assert!(!QT::HOOKS_DO_SIDE_EFFECTS, "When using QemuForkExecutor, the hooks must not do any side effect as they will happen in the child process and then discarded");

        Ok(Self {
            inner: InProcessForkExecutor::new(
                harness_fn,
                observers,
                fuzzer,
                state,
                event_mgr,
                timeout,
                shmem_provider,
            )?,
            state: QemuExecutorState {
                first_exec: true,
                hooks,
            },
        })
    }

    pub fn inner(&self) -> &InProcessForkExecutor<'a, H, OT, S, SP, EM, Z> {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut InProcessForkExecutor<'a, H, OT, S, SP, EM, Z> {
        &mut self.inner
    }

    pub fn hooks(&self) -> &QemuHooks<QT, S, E> {
        self.state.hooks
    }

    pub fn hooks_mut(&mut self) -> &mut QemuHooks<QT, S, E> {
        self.state.hooks
    }

    pub fn emulator(&self) -> &Emulator<E> {
        self.state.hooks.emulator()
    }
}

#[cfg(feature = "fork")]
impl<'a, EM, H, OT, QT, S, Z, SP, OF, E> Executor<EM, Z>
    for QemuForkExecutor<'a, H, OT, QT, S, SP, E, EM, Z>
where
    EM: EventManager<InProcessForkExecutor<'a, H, OT, S, SP, EM, Z>, Z, State = S>,
    H: FnMut(&S::Input) -> ExitKind,
    S: State + HasMetadata + HasExecutions + HasLastReportTime + HasCorpus + HasSolutions,
    OT: ObserversTuple<S> + Debug,
    QT: QemuHelperTuple<S, E>,
    E: IsEmuExitHandler,
    SP: ShMemProvider,
    OF: Feedback<S>,
    Z: HasObjective<Objective = OF, State = S>,
{
    fn run_target(
        &mut self,
        fuzzer: &mut Z,
        state: &mut Self::State,
        mgr: &mut EM,
        input: &Self::Input,
    ) -> Result<ExitKind, Error> {
        let emu = self.state.hooks.emulator().clone();
        if self.state.first_exec {
            self.state.hooks.helpers().first_exec_all(self.state.hooks);
            self.state.first_exec = false;
        }
        self.state.hooks.helpers_mut().pre_exec_all(&emu, input);
        let mut exit_kind = self.inner.run_target(fuzzer, state, mgr, input)?;
        self.state.hooks.helpers_mut().post_exec_all(
            &emu,
            input,
            self.inner.observers_mut(),
            &mut exit_kind,
        );
        Ok(exit_kind)
    }
}

#[cfg(feature = "fork")]
impl<'a, H, OT, QT, S, SP, E, EM, Z> UsesObservers
    for QemuForkExecutor<'a, H, OT, QT, S, SP, E, EM, Z>
where
    H: FnMut(&S::Input) -> ExitKind,
    OT: ObserversTuple<S>,
    QT: QemuHelperTuple<S, E>,
    S: State + HasExecutions,
    SP: ShMemProvider,
    E: IsEmuExitHandler,
    EM: UsesState<State = S>,
    Z: UsesState<State = S>,
{
    type Observers = OT;
}

#[cfg(feature = "fork")]
impl<'a, H, OT, QT, S, SP, E, EM, Z> UsesState for QemuForkExecutor<'a, H, OT, QT, S, SP, E, EM, Z>
where
    H: FnMut(&S::Input) -> ExitKind,
    OT: ObserversTuple<S>,
    QT: QemuHelperTuple<S, E>,
    S: State + HasExecutions,
    SP: ShMemProvider,
    E: IsEmuExitHandler,
    EM: UsesState<State = S>,
    Z: UsesState<State = S>,
{
    type State = S;
}

#[cfg(feature = "fork")]
impl<'a, H, OT, QT, S, SP, E, EM, Z> HasObservers
    for QemuForkExecutor<'a, H, OT, QT, S, SP, E, EM, Z>
where
    H: FnMut(&S::Input) -> ExitKind,
    S: State + HasExecutions,
    OT: ObserversTuple<S>,
    QT: QemuHelperTuple<S, E>,
    SP: ShMemProvider,
    E: IsEmuExitHandler,
    EM: UsesState<State = S>,
    Z: UsesState<State = S>,
{
    #[inline]
    fn observers(&self) -> &OT {
        self.inner.observers()
    }

    #[inline]
    fn observers_mut(&mut self) -> &mut OT {
        self.inner.observers_mut()
    }
}