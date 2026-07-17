use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashSet},
    net::IpAddr,
    num::NonZeroU32,
    sync::Arc,
    time::Instant,
};

/// The DNS-SD `(name, service type, domain)` identity a device advertises. A
/// structured key (rather than a joined string) so that separator characters
/// appearing in a service name cannot make two different registrations compare
/// equal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Registration {
    pub name: String,
    pub service_type: String,
    pub domain: String,
}

impl Registration {
    pub fn new(
        name: impl Into<String>,
        service_type: impl Into<String>,
        domain: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            service_type: service_type.into(),
            domain: domain.into(),
        }
    }
}

/// A discovery adapter's own name for one occurrence of a registration. Its
/// meaning belongs to the adapter that issued it — the mdns-sd adapter uses the
/// network interface an announcement arrived on — and discovery asks only that
/// the same occurrence keep the same value across the events describing it.
///
/// Adapters that cannot name their occurrences (the zeroconf adapter) issue
/// none. The value is an identity, not a fact about the network: it must not
/// reach display labels or command fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OccurrenceId(pub NonZeroU32);

/// What tells one occurrence of a registration apart from another.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Occurrence {
    /// The adapter named this occurrence. The name is stable across updates, so
    /// the occurrence keeps its identity when its endpoint, addresses, or TXT
    /// data change.
    Named(OccurrenceId),
    /// No adapter name: the resolved SRV endpoint is all that keeps concurrent
    /// occurrences of one registration apart. A service's address set can
    /// change without it becoming a different service, so addresses stay out.
    Endpoint { hostname: String, port: Option<u16> },
    /// Announced, but nothing resolved yet — a placeholder for the
    /// registration rather than an occurrence of it.
    Pending,
}

/// Identity of an [`Entry`]: which registration it belongs to, and which
/// occurrence of that registration it is. Two occurrences of one registration
/// (the same service announced on two interfaces, say) have different ids and
/// coexist in a record store; the UI may still group them for display.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryId {
    registration: Registration,
    occurrence: Occurrence,
}

impl EntryId {
    /// The id of the occurrence an adapter named `occurrence`.
    pub fn named(registration: Registration, occurrence: OccurrenceId) -> Self {
        Self {
            registration,
            occurrence: Occurrence::Named(occurrence),
        }
    }

    /// The id of the unresolved placeholder for `registration`: what an adapter
    /// emits once it has seen the announcement but resolved no occurrence of it.
    /// Superseded as soon as any real occurrence arrives.
    pub fn pending(registration: Registration) -> Self {
        Self {
            registration,
            occurrence: Occurrence::Pending,
        }
    }

    /// The registration every occurrence id of one registration shares.
    pub fn registration(&self) -> &Registration {
        &self.registration
    }

    /// Whether this id is a registration's unresolved placeholder rather than
    /// an occurrence of it.
    pub fn is_pending(&self) -> bool {
        matches!(self.occurrence, Occurrence::Pending)
    }
}

/// The label shown for the row that collects registrations which have not
/// resolved a host yet. It is display text only: [`HostKey::Unresolved`], not
/// this string, is what groups those registrations, so a device advertising
/// this exact hostname cannot join them.
pub const UNRESOLVED_HOST_LABEL: &str = "<unresolved host>";

/// Which host a row belongs to. A variant rather than an `Option<String>` so
/// that "no host resolved yet" is a distinct identity from every hostname a
/// device could advertise, including one equal to [`UNRESOLVED_HOST_LABEL`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostKey {
    Resolved(String),
    /// Ordered after every resolved host, which settles the order when a device
    /// advertises [`UNRESOLVED_HOST_LABEL`] as its hostname and the two rows'
    /// labels are identical.
    Unresolved,
}

impl HostKey {
    fn of(hostname: Option<&str>) -> Self {
        match hostname {
            Some(host) => HostKey::Resolved(host.to_string()),
            None => HostKey::Unresolved,
        }
    }
}

/// Identity of one row of a browse projection.
///
/// Structured, not a joined string: a separator occurring inside a service name
/// cannot shift a boundary and make two different rows compare equal — the same
/// reasoning that gave [`EntryId`] a structured key. Being `Ord`, it is also the
/// final sort key that makes row order total when labels collide, and the handle
/// the UI re-finds a selected row by after a recomputation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryGroupId {
    /// A logical service: everything its occurrences must agree on to be one
    /// user-facing service.
    LogicalService {
        name: String,
        service_type: String,
        domain: String,
        host: HostKey,
        port: Option<u16>,
    },
    /// One host, or the single row collecting the registrations that have not
    /// resolved a host yet.
    Host(HostKey),
    /// One DNS-SD service type.
    ServiceType(String),
}

/// A projection of discovered entries into browsable rows.
///
/// Each variant has its own invariants, and [`browse_groups`] gives a row only
/// the facts that hold for every occurrence it aggregates. A host row has no
/// one service type, port, or TXT set; a service-type row has no one host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BrowseMode {
    LogicalService,
    Host,
    ServiceType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GroupingMode {
    LogicalService,
    Host,
    ServiceType,
    Command,
}

impl GroupingMode {
    /// The browse projection behind this tab, or `None` for
    /// [`GroupingMode::Command`] — which lists configured rules rather than a
    /// projection of discovered entries, and reuses logical-service rows as its
    /// children.
    pub fn browse_mode(self) -> Option<BrowseMode> {
        match self {
            GroupingMode::LogicalService => Some(BrowseMode::LogicalService),
            GroupingMode::Host => Some(BrowseMode::Host),
            GroupingMode::ServiceType => Some(BrowseMode::ServiceType),
            GroupingMode::Command => None,
        }
    }
}

impl GroupingMode {
    /// The grouping modes surfaced as top-panel tabs, in display order. The
    /// first entry is the default view shown at startup.
    pub const TABS: [GroupingMode; 4] = [
        GroupingMode::LogicalService,
        GroupingMode::Host,
        GroupingMode::ServiceType,
        GroupingMode::Command,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GroupingMode::LogicalService => "logical service",
            GroupingMode::Host => "host",
            GroupingMode::ServiceType => "service type",
            GroupingMode::Command => "command",
        }
    }

    /// Short label shown on the top-panel tab for this view.
    pub fn tab_title(self) -> &'static str {
        match self {
            GroupingMode::LogicalService => "services",
            GroupingMode::Host => "hosts",
            GroupingMode::ServiceType => "types",
            GroupingMode::Command => "commands",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub service_type: String,
    pub domain: String,
    pub hostname: Option<String>,
    /// All IP addresses the service's host resolved to. A service may advertise
    /// several (e.g. IPv4 + IPv6, or DNS load-balanced A records); they are kept
    /// together on the single logical-service entry. The first is the primary.
    pub addresses: Vec<IpAddr>,
    pub port: Option<u16>,
    pub txt: BTreeMap<String, String>,
    pub last_seen: Instant,
    /// The adapter's name for this occurrence, when it has one. Private and
    /// only settable at construction: [`Entry::id`] is derived from it on
    /// demand, so no caller has to keep a stored id in step with the fields
    /// that define it.
    occurrence: Option<OccurrenceId>,
}

impl Entry {
    pub fn new(
        name: impl Into<String>,
        service_type: impl Into<String>,
        domain: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            service_type: service_type.into(),
            domain: domain.into(),
            hostname: None,
            addresses: Vec::new(),
            port: None,
            txt: BTreeMap::new(),
            last_seen: Instant::now(),
            occurrence: None,
        }
    }

    /// Build a resolved entry from the fields a discovery backend reports.
    /// Blank hostnames and zero ports count as "not resolved yet". Shared by
    /// every backend so they agree on what an unresolved field looks like.
    pub fn resolved(
        name: &str,
        service_type: &str,
        domain: &str,
        hostname: Option<&str>,
        addresses: Vec<IpAddr>,
        port: Option<u16>,
        txt: BTreeMap<String, String>,
    ) -> Self {
        let mut record = Entry::new(name, service_type, domain);
        record.hostname = hostname
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        record.addresses = addresses;
        record.port = port.filter(|port| *port != 0);
        record.txt = txt;
        record
    }

    /// Name the occurrence this entry describes. Adapters that can tell their
    /// occurrences apart call this so that updates and removals address exactly
    /// one of them; adapters that cannot pass `None` and fall back to endpoint
    /// identity.
    pub fn with_occurrence(mut self, occurrence: Option<OccurrenceId>) -> Self {
        self.occurrence = occurrence;
        self
    }

    /// The adapter's name for this occurrence, if it gave one.
    pub fn occurrence(&self) -> Option<OccurrenceId> {
        self.occurrence
    }

    /// This entry's identity, derived from its current fields. An adapter's own
    /// occurrence name wins when present: it survives the endpoint changing, so
    /// re-resolving one occurrence updates it in place instead of forking a
    /// duplicate. Without one, the resolved endpoint discriminates.
    pub fn id(&self) -> EntryId {
        let occurrence = match (self.occurrence, self.has_instance_data()) {
            (Some(named), _) => Occurrence::Named(named),
            (None, true) => Occurrence::Endpoint {
                hostname: self.hostname.clone().unwrap_or_default(),
                port: self.port,
            },
            (None, false) => Occurrence::Pending,
        };
        EntryId {
            registration: self.registration(),
            occurrence,
        }
    }

    /// The registration this entry is an occurrence of.
    pub fn registration(&self) -> Registration {
        Registration::new(
            self.name.clone(),
            self.service_type.clone(),
            self.domain.clone(),
        )
    }

    pub fn has_instance_data(&self) -> bool {
        self.hostname.is_some() || !self.addresses.is_empty() || self.port.is_some()
    }

    /// The primary (first) address, used wherever a single IP is needed
    /// (command templating, sorting, compact display).
    pub fn primary_address(&self) -> Option<IpAddr> {
        self.addresses.first().copied()
    }

    /// Look up a command/matcher field without copying text already owned by
    /// this entry. Typed fields still format into an owned value.
    pub(crate) fn field_value(&self, field: &str) -> Option<Cow<'_, str>> {
        match field {
            "name" => Some(Cow::Borrowed(&self.name)),
            "type" | "service_type" => Some(Cow::Borrowed(&self.service_type)),
            "domain" => Some(Cow::Borrowed(&self.domain)),
            "hostname" => self.hostname.as_deref().map(Cow::Borrowed),
            "address" => self
                .primary_address()
                .map(|value| Cow::Owned(value.to_string())),
            "port" => self.port.map(|value| Cow::Owned(value.to_string())),
            // `strip_prefix`, not `trim_start_matches`: the latter strips the
            // prefix repeatedly, so `txt.txt.path` would look up `path`
            // instead of the TXT key literally named `txt.path`.
            field => field.strip_prefix("txt.").and_then(|key| {
                self.txt
                    .get(key)
                    .or_else(|| {
                        self.txt.iter().find_map(|(candidate, value)| {
                            candidate.eq_ignore_ascii_case(key).then_some(value)
                        })
                    })
                    .map(|value| Cow::Borrowed(value.as_str()))
            }),
        }
    }

    /// Whether this entry carries `field`, without formatting or cloning its
    /// value. Rule eligibility asks this much more often than it renders.
    pub(crate) fn has_field(&self, field: &str) -> bool {
        match field {
            "name" | "type" | "service_type" | "domain" => true,
            "hostname" => self.hostname.is_some(),
            "address" => self.primary_address().is_some(),
            "port" => self.port.is_some(),
            field => field.strip_prefix("txt.").is_some_and(|key| {
                self.txt.contains_key(key)
                    || self
                        .txt
                        .keys()
                        .any(|candidate| candidate.eq_ignore_ascii_case(key))
            }),
        }
    }

    pub fn field(&self, field: &str) -> Option<String> {
        self.field_value(field).map(Cow::into_owned)
    }

    pub fn searchable_text(&self) -> String {
        let mut text = format!(
            "{} {} {} {}",
            self.name,
            self.display_name(),
            self.service_type,
            self.domain
        );
        if let Some(hostname) = &self.hostname {
            text.push(' ');
            text.push_str(hostname);
        }
        for address in &self.addresses {
            text.push(' ');
            text.push_str(&address.to_string());
        }
        if let Some(port) = self.port {
            text.push(' ');
            text.push_str(&port.to_string());
        }
        for (key, value) in &self.txt {
            text.push(' ');
            text.push_str(key);
            text.push(' ');
            text.push_str(value);
        }
        text
    }

    pub fn display_name(&self) -> String {
        decode_dns_sd_escapes(&self.name)
    }

    /// The display name with Avahi's per-interface ` [aa:bb:cc:dd:ee:ff]` MAC
    /// decoration stripped. A multi-homed host publishing the same service on
    /// several interfaces (e.g. avahi-daemon's workstation publisher) uses one
    /// instance name per interface, differing only in this suffix; the base
    /// name is what identifies the service to a person. Bracket suffixes that
    /// are not a MAC address (e.g. a printer's `[9917FB]` serial) are kept.
    pub fn base_display_name(&self) -> String {
        let display = self.display_name();
        match display.rsplit_once(" [") {
            Some((base, rest)) if !base.is_empty() && is_mac_suffix(rest) => base.to_string(),
            _ => display,
        }
    }
}

/// Whether `rest` (the text following ` [`) is a MAC address plus the closing
/// bracket: six pairs of hex digits separated by colons.
fn is_mac_suffix(rest: &str) -> bool {
    let Some(mac) = rest.strip_suffix(']') else {
        return false;
    };
    let mut groups = 0;
    for group in mac.split(':') {
        if group.len() != 2 || !group.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return false;
        }
        groups += 1;
    }
    groups == 6
}

/// The facts every occurrence of a logical-service row shares.
///
/// These are exactly the fields the row's identity is built from, so none of
/// them can be an arbitrary occurrence's value. Data that genuinely varies
/// between occurrences — addresses and TXT records — is deliberately absent:
/// ask the row's occurrences, or [`EntryGroup::txt`], for those.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalService {
    /// The base display name every occurrence shares; the row's label.
    pub name: String,
    pub service_type: String,
    pub domain: String,
    /// The host every occurrence resolved to, or `None` while unresolved.
    pub hostname: Option<String>,
    pub port: Option<u16>,
}

/// The facts every occurrence of a host row shares: the host, and nothing else.
///
/// Service type, port, domain, and TXT data belong to the row's child services,
/// which may disagree about all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostAggregate {
    /// `None` for the row collecting registrations with no resolved host yet.
    pub hostname: Option<String>,
}

/// The facts every occurrence of a service-type row shares: the type, and
/// nothing else. Host, port, and TXT data belong to the row's child services.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceTypeAggregate {
    pub service_type: String,
}

/// A row's facts, in the shape its projection can vouch for.
///
/// One variant per [`BrowseMode`], so a caller cannot read a field the active
/// projection does not guarantee — the aggregate views have no service type,
/// port, or TXT set to read at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupFacts {
    LogicalService(LogicalService),
    Host(HostAggregate),
    ServiceType(ServiceTypeAggregate),
}

/// What a row can truthfully say about the host of its occurrences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowHost<'a> {
    /// Every occurrence in the row is on this host.
    Resolved(&'a str),
    /// Every occurrence in the row is on one host, which has not resolved a
    /// name yet.
    Unresolved,
    /// The row aggregates several hosts, so it has no host-wide hostname and
    /// host-scoped operations (the same-host filter) are unavailable.
    Varies,
}

/// What a row can truthfully say about the DNS-SD service type of its
/// occurrences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowServiceType<'a> {
    /// Every occurrence in the row advertises this type.
    Invariant(&'a str),
    /// The row aggregates several service types.
    Varies,
}

impl GroupFacts {
    /// The row's label. Derived from the facts rather than stored beside them,
    /// so the text and the facts cannot disagree.
    pub fn label(&self) -> String {
        match self {
            GroupFacts::LogicalService(service) => service.name.clone(),
            GroupFacts::Host(host) => host
                .hostname
                .clone()
                .unwrap_or_else(|| UNRESOLVED_HOST_LABEL.to_string()),
            GroupFacts::ServiceType(aggregate) => aggregate.service_type.clone(),
        }
    }

    pub fn host(&self) -> RowHost<'_> {
        let hostname = match self {
            GroupFacts::LogicalService(service) => service.hostname.as_deref(),
            GroupFacts::Host(host) => host.hostname.as_deref(),
            // A service type is offered by whoever offers it.
            GroupFacts::ServiceType(_) => return RowHost::Varies,
        };
        match hostname {
            Some(host) => RowHost::Resolved(host),
            None => RowHost::Unresolved,
        }
    }

    pub fn service_type(&self) -> RowServiceType<'_> {
        match self {
            GroupFacts::LogicalService(service) => RowServiceType::Invariant(&service.service_type),
            GroupFacts::ServiceType(aggregate) => {
                RowServiceType::Invariant(&aggregate.service_type)
            }
            // A host offers whatever it offers.
            GroupFacts::Host(_) => RowServiceType::Varies,
        }
    }
}

/// A TXT key's value across the occurrences of a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxtValue {
    /// Every occurrence carries this key with this value.
    Shared(String),
    /// The occurrences disagree, or only some carry the key at all. No single
    /// value describes the row, and saying so is the only honest answer.
    Mixed,
}

/// A logical service listed inside an aggregate row.
///
/// Display facts and a count only: the occurrences themselves stay owned by the
/// row, so a child list can never drift from the entries it describes. Actions
/// target the row's occurrences, never a child summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildService {
    pub id: EntryGroupId,
    pub facts: LogicalService,
    pub occurrences: usize,
}

/// One row of a browse projection: its identity, the facts valid for its
/// projection, and the concrete occurrences it aggregates.
///
/// The occurrences are the row's only entry collection. Labels, counts, child
/// lists, and TXT views are derived from them or from the row's identity on
/// demand, never stored alongside where they could fall out of step.
#[derive(Debug, Clone)]
pub struct EntryGroup {
    id: EntryGroupId,
    label: String,
    facts: GroupFacts,
    instances: Arc<[Entry]>,
}

impl EntryGroup {
    /// The row's structured identity: stable across recomputation and unique
    /// within a projection, so the UI can re-find a selected row by it.
    pub fn id(&self) -> &EntryGroupId {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// The facts this row's projection can vouch for.
    pub fn facts(&self) -> &GroupFacts {
        &self.facts
    }

    /// The concrete occurrences this row aggregates. Matching and invocation
    /// use these; the row's aggregate facts describe them but never stand in
    /// for one of them.
    pub fn instances(&self) -> &[Entry] {
        &self.instances
    }

    pub fn occurrence_count(&self) -> usize {
        self.instances.len()
    }

    /// Distinct logical services among this row's occurrences.
    pub fn logical_service_count(&self) -> usize {
        browse_row_count(&self.instances, BrowseMode::LogicalService)
    }

    /// Distinct hosts among the occurrences that have resolved one. The
    /// unresolved occurrences are deliberately uncounted: they are not a host.
    pub fn resolved_host_count(&self) -> usize {
        self.instances
            .iter()
            .filter_map(|record| record.hostname.as_deref())
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// The distinct service types across this row's occurrences, sorted. A host
    /// row may offer several; a logical-service or service-type row has one.
    pub fn service_types(&self) -> Vec<String> {
        self.instances
            .iter()
            .map(|record| record.service_type.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// The logical services this row aggregates, in list order.
    pub fn child_services(&self) -> Vec<ChildService> {
        let mut buckets: BTreeMap<EntryGroupId, (LogicalService, usize)> = BTreeMap::new();
        for record in self.instances.iter() {
            let (id, facts) = logical_service_of(record);
            let child = buckets.entry(id).or_insert_with(|| (facts, 0));
            child.1 += 1;
        }
        let mut children: Vec<ChildService> = buckets
            .into_iter()
            .map(|(id, (facts, occurrences))| ChildService {
                id,
                facts,
                occurrences,
            })
            .collect();
        children.sort_by(|a, b| {
            a.facts
                .name
                .cmp(&b.facts.name)
                .then_with(|| a.id.cmp(&b.id))
        });
        children
    }

    /// This row's TXT data, over the union of its occurrences' keys.
    ///
    /// A key every occurrence carries with one value is [`TxtValue::Shared`];
    /// anything else is [`TxtValue::Mixed`], because no single value describes
    /// the row. Only the logical-service view shows this: an aggregate row
    /// collects unrelated services, whose TXT keys mean nothing side by side.
    pub fn txt(&self) -> BTreeMap<String, TxtValue> {
        let mut merged: BTreeMap<String, TxtValue> = BTreeMap::new();
        for record in self.instances.iter() {
            for (key, value) in &record.txt {
                match merged.get(key) {
                    None => {
                        merged.insert(key.clone(), TxtValue::Shared(value.clone()));
                    }
                    Some(TxtValue::Shared(existing)) if existing != value => {
                        merged.insert(key.clone(), TxtValue::Mixed);
                    }
                    Some(TxtValue::Shared(_) | TxtValue::Mixed) => {}
                }
            }
        }
        // A key only some occurrences carry is not row-wide either.
        for (key, value) in merged.iter_mut() {
            if self
                .instances
                .iter()
                .any(|record| !record.txt.contains_key(key))
            {
                *value = TxtValue::Mixed;
            }
        }
        merged
    }
}

/// Project `records` onto the rows of `mode`, in display order.
///
/// Each row carries only the facts its projection guarantees, plus every
/// occurrence behind it.
pub fn browse_groups(records: &[Entry], mode: BrowseMode) -> Vec<EntryGroup> {
    let mut buckets: BTreeMap<EntryGroupId, (GroupFacts, Vec<Entry>)> = BTreeMap::new();
    for record in records {
        let (id, facts) = row_of(record, mode);
        buckets
            .entry(id)
            .or_insert_with(|| (facts, Vec::new()))
            .1
            .push(record.clone());
    }

    groups_from_buckets(buckets)
}

/// The active browse rows plus all three browse-tab counts, computed while
/// visiting each filtered entry once.
pub(crate) struct BrowseProjection {
    pub(crate) groups: Vec<EntryGroup>,
    pub(crate) counts: [usize; 3],
}

pub(crate) fn browse_projection(records: &[Entry], active: BrowseMode) -> BrowseProjection {
    const MODES: [BrowseMode; 3] = [
        BrowseMode::LogicalService,
        BrowseMode::Host,
        BrowseMode::ServiceType,
    ];

    let active_index = browse_mode_index(active);
    let mut identities: [HashSet<EntryGroupId>; 3] = std::array::from_fn(|_| HashSet::new());
    let mut buckets: BTreeMap<EntryGroupId, (GroupFacts, Vec<Entry>)> = BTreeMap::new();

    for record in records {
        for (index, mode) in MODES.into_iter().enumerate() {
            let (id, facts) = row_of(record, mode);
            if index == active_index {
                buckets
                    .entry(id.clone())
                    .or_insert_with(|| (facts, Vec::new()))
                    .1
                    .push(record.clone());
            }
            identities[index].insert(id);
        }
    }

    BrowseProjection {
        groups: groups_from_buckets(buckets),
        counts: identities.map(|ids| ids.len()),
    }
}

fn browse_mode_index(mode: BrowseMode) -> usize {
    match mode {
        BrowseMode::LogicalService => 0,
        BrowseMode::Host => 1,
        BrowseMode::ServiceType => 2,
    }
}

fn groups_from_buckets(
    buckets: BTreeMap<EntryGroupId, (GroupFacts, Vec<Entry>)>,
) -> Vec<EntryGroup> {
    let mut groups: Vec<EntryGroup> = buckets
        .into_iter()
        .map(|(id, (facts, mut instances))| {
            instances.sort_by(compare_occurrences);
            EntryGroup {
                label: facts.label(),
                id,
                facts,
                instances: instances.into(),
            }
        })
        .collect();

    // Label first — that is the order a reader perceives — then the structured
    // identity, which is unique per row. The order is therefore total: rows
    // with duplicate labels keep a fixed sequence across recomputations rather
    // than shuffling with the input.
    groups.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.id.cmp(&b.id)));
    groups
}

/// How many rows the `mode` projection of `records` has, without building them.
/// Tab counts read this, so a tab's count and its list can never disagree.
pub fn browse_row_count(records: &[Entry], mode: BrowseMode) -> usize {
    records
        .iter()
        .map(|record| row_of(record, mode).0)
        .collect::<BTreeSet<_>>()
        .len()
}

/// The row `record` belongs to under `mode`: its identity, and the facts every
/// occurrence of that row shares.
///
/// Identity and facts are built here from the same fields, which is what makes
/// a row's facts true of all its occurrences instead of copied from one of
/// them: any two records in a bucket produce identical facts by construction.
fn row_of(record: &Entry, mode: BrowseMode) -> (EntryGroupId, GroupFacts) {
    match mode {
        BrowseMode::LogicalService => {
            let (id, facts) = logical_service_of(record);
            (id, GroupFacts::LogicalService(facts))
        }
        BrowseMode::Host => (
            EntryGroupId::Host(HostKey::of(record.hostname.as_deref())),
            GroupFacts::Host(HostAggregate {
                hostname: record.hostname.clone(),
            }),
        ),
        BrowseMode::ServiceType => (
            EntryGroupId::ServiceType(record.service_type.clone()),
            GroupFacts::ServiceType(ServiceTypeAggregate {
                service_type: record.service_type.clone(),
            }),
        ),
    }
}

/// The logical service `record` is an occurrence of: its identity and shared
/// facts.
///
/// The base display name keys it, not the raw instance name: a multi-homed
/// host's per-interface instances — same service, names differing only in
/// Avahi's ` [MAC]` decoration — are one logical service whose occurrences
/// carry the per-interface addresses.
fn logical_service_of(record: &Entry) -> (EntryGroupId, LogicalService) {
    let facts = LogicalService {
        name: record.base_display_name(),
        service_type: record.service_type.clone(),
        domain: record.domain.clone(),
        hostname: record.hostname.clone(),
        port: record.port,
    };
    let id = EntryGroupId::LogicalService {
        name: facts.name.clone(),
        service_type: facts.service_type.clone(),
        domain: facts.domain.clone(),
        host: HostKey::of(facts.hostname.as_deref()),
        port: facts.port,
    };
    (id, facts)
}

/// Total order over the occurrences within one row: the fields a reader sees
/// first, then the occurrence identity, which is unique. Two occurrences of one
/// registration can agree on every visible field and differ only in who named
/// them; ordering by identity last keeps the rendered list from shuffling.
fn compare_occurrences(a: &Entry, b: &Entry) -> Ordering {
    a.name
        .cmp(&b.name)
        .then_with(|| a.service_type.cmp(&b.service_type))
        .then_with(|| a.hostname.cmp(&b.hostname))
        .then_with(|| a.addresses.cmp(&b.addresses))
        .then_with(|| a.port.cmp(&b.port))
        .then_with(|| a.id().cmp(&b.id()))
}

pub fn decode_dns_sd_escapes(value: &str) -> String {
    let mut bytes = Vec::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
            continue;
        }

        let mut digits = String::new();
        for _ in 0..3 {
            let Some(next) = chars.peek().copied() else {
                break;
            };
            if !next.is_ascii_digit() {
                break;
            }
            digits.push(next);
            chars.next();
        }

        if digits.len() == 3
            && let Ok(byte) = digits.parse::<u8>()
        {
            bytes.push(byte);
            continue;
        }

        bytes.push(b'\\');
        bytes.extend_from_slice(digits.as_bytes());
    }

    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_builds_record_from_browse_fields() {
        let mut txt = BTreeMap::new();
        txt.insert("path".to_string(), "/admin".to_string());

        let record = Entry::resolved(
            "nas",
            "_http._tcp",
            "local",
            Some("nas.local"),
            vec!["192.168.1.30".parse().unwrap()],
            Some(8080),
            txt,
        );

        assert_eq!(record.name, "nas");
        assert_eq!(record.hostname.as_deref(), Some("nas.local"));
        assert_eq!(
            record.primary_address(),
            Some("192.168.1.30".parse().unwrap())
        );
        assert_eq!(record.port, Some(8080));
        assert_eq!(record.txt.get("path").map(String::as_str), Some("/admin"));
        assert!(record.has_instance_data());
    }

    #[test]
    fn resolved_treats_blank_host_and_zero_port_as_unresolved() {
        let record = Entry::resolved(
            "pending",
            "_ipp._tcp",
            "local",
            Some(""),
            Vec::new(),
            Some(0),
            BTreeMap::new(),
        );

        assert_eq!(record.hostname, None);
        assert!(record.addresses.is_empty());
        assert_eq!(record.port, None);
        assert!(!record.has_instance_data());
    }

    #[test]
    fn logical_service_keeps_all_its_addresses_on_one_entry() {
        let mut entry = Entry::new("host", "_ssh._tcp", "local");
        entry.hostname = Some("host.local".to_string());
        entry.addresses = vec![
            "192.168.1.10".parse().unwrap(),
            "192.168.1.11".parse().unwrap(),
        ];
        entry.port = Some(22);

        let groups = browse_groups(&[entry], BrowseMode::LogicalService);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].instances().len(), 1);
        assert_eq!(groups[0].instances()[0].addresses.len(), 2);
    }

    #[test]
    fn display_name_decodes_avahi_decimal_escapes() {
        let record = Entry::new(
            r"HP\032OfficeJet\032Pro\0328020\032series\032\0919917FB\093",
            "_ipp._tcp",
            "local",
        );

        assert_eq!(
            record.display_name(),
            "HP OfficeJet Pro 8020 series [9917FB]"
        );
        assert_eq!(
            record.name,
            r"HP\032OfficeJet\032Pro\0328020\032series\032\0919917FB\093"
        );
    }

    #[test]
    fn display_name_decodes_utf8_byte_escapes() {
        let record = Entry::new(r"Caf\195\169", "_http._tcp", "local");

        assert_eq!(record.display_name(), "Café");
    }

    #[test]
    fn base_display_name_strips_interface_mac_decoration() {
        // Raw Avahi escapes for `rpi5-0 [d8:3a:dd:f4:b1:dc]`.
        let per_interface = Entry::new(
            r"rpi5-0\032\091d8\0583a\058dd\058f4\058b1\058dc\093",
            "_workstation._tcp",
            "local",
        );
        assert_eq!(per_interface.base_display_name(), "rpi5-0");

        // A non-MAC bracket suffix (printer serial) is not decoration.
        let printer = Entry::new(r"HP\032Printer\032\0919917FB\093", "_ipp._tcp", "local");
        assert_eq!(printer.base_display_name(), "HP Printer [9917FB]");

        // Plain names and near-misses are untouched.
        assert_eq!(
            Entry::new("plain", "_ssh._tcp", "local").base_display_name(),
            "plain"
        );
        assert_eq!(
            Entry::new("x [d8:3a:dd:f4:b1]", "_ssh._tcp", "local").base_display_name(),
            "x [d8:3a:dd:f4:b1]"
        );
        assert_eq!(
            Entry::new("[d8:3a:dd:f4:b1:dc]", "_ssh._tcp", "local").base_display_name(),
            "[d8:3a:dd:f4:b1:dc]"
        );
    }

    #[test]
    fn per_interface_instances_merge_into_one_logical_service() {
        let mut wired = Entry::new("rpi5-0 [d8:3a:dd:f4:b1:dc]", "_workstation._tcp", "local");
        wired.hostname = Some("rpi5-0.local".to_string());
        wired.addresses = vec!["192.168.50.244".parse().unwrap()];
        wired.port = Some(9);
        let mut wireless = Entry::new("rpi5-0 [d8:3a:dd:f4:b1:dd]", "_workstation._tcp", "local");
        wireless.hostname = Some("rpi5-0.local".to_string());
        wireless.addresses = vec!["192.168.50.245".parse().unwrap()];
        wireless.port = Some(9);

        let groups = browse_groups(&[wired, wireless], BrowseMode::LogicalService);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].label(), "rpi5-0");
        assert_eq!(groups[0].instances().len(), 2);
    }

    #[test]
    fn different_services_sharing_an_endpoint_stay_separate() {
        // Two genuinely different services (e.g. HTTP virtual hosts) on the
        // same host and port must not be merged by the base-name grouping.
        let mut site_a = Entry::new("Site A", "_http._tcp", "local");
        site_a.hostname = Some("nas.local".to_string());
        site_a.port = Some(80);
        let mut site_b = Entry::new("Site B", "_http._tcp", "local");
        site_b.hostname = Some("nas.local".to_string());
        site_b.port = Some(80);

        let groups = browse_groups(&[site_a, site_b], BrowseMode::LogicalService);

        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn logical_group_label_uses_decoded_display_name() {
        let record = Entry::new(r"HP\032Printer", "_ipp._tcp", "local");

        let groups = browse_groups(&[record], BrowseMode::LogicalService);

        assert_eq!(groups[0].label(), "HP Printer");
    }

    fn resolved(name: &str, service_type: &str) -> Entry {
        let mut record = Entry::new(name, service_type, "local");
        record.hostname = Some(format!("{name}.local"));
        record.addresses = vec!["192.168.1.10".parse().unwrap()];
        record.port = Some(22);
        record
    }

    #[test]
    fn field_exposes_all_supported_keys_and_aliases() {
        let mut record = Entry::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        record.addresses = vec!["192.0.2.5".parse().unwrap()];
        record.port = Some(22);
        record.txt.insert("path".to_string(), "/admin".to_string());

        assert_eq!(record.field("name").as_deref(), Some("alpha"));
        assert_eq!(record.field("type").as_deref(), Some("_ssh._tcp"));
        assert_eq!(record.field("service_type").as_deref(), Some("_ssh._tcp"));
        assert_eq!(record.field("domain").as_deref(), Some("local"));
        assert_eq!(record.field("hostname").as_deref(), Some("alpha.local"));
        assert_eq!(record.field("address").as_deref(), Some("192.0.2.5"));
        assert_eq!(record.field("port").as_deref(), Some("22"));
        assert_eq!(record.field("txt.path").as_deref(), Some("/admin"));
        assert_eq!(record.field("txt.missing"), None);
        assert_eq!(record.field("unknown"), None);
    }

    #[test]
    fn pending_record_has_no_occurrence_identity_until_resolved() {
        let pending = Entry::new("alpha", "_ssh._tcp", "local");
        assert_eq!(pending.id(), EntryId::pending(pending.registration()));
        assert!(pending.id().is_pending());
        assert!(!pending.has_instance_data());

        // Resolving gives the entry an endpoint, so it is an occurrence of the
        // registration rather than a placeholder for it.
        let resolved = resolved("alpha", "_ssh._tcp");
        assert!(!resolved.id().is_pending());
        assert_ne!(resolved.id(), pending.id());
        assert!(resolved.has_instance_data());
    }

    #[test]
    fn registration_is_the_name_type_domain_triple() {
        let resolved = resolved("alpha", "_ssh._tcp");
        assert_eq!(
            *resolved.id().registration(),
            Registration::new("alpha", "_ssh._tcp", "local")
        );
        // Every occurrence id of one registration reports the same registration.
        assert_eq!(
            resolved.id().registration(),
            &Entry::new("alpha", "_ssh._tcp", "local").registration()
        );
    }

    fn on_interface(name: &str, addr: &str, index: u32) -> Entry {
        let mut record = Entry::new(name, "_ssh._tcp", "local");
        record.hostname = Some(format!("{name}.local"));
        record.addresses = vec![addr.parse().unwrap()];
        record.port = Some(22);
        record.with_occurrence(Some(OccurrenceId(NonZeroU32::new(index).unwrap())))
    }

    #[test]
    fn occurrences_differing_only_by_adapter_name_are_not_equal() {
        let wired = on_interface("alpha", "10.0.0.1", 1);
        let wireless = on_interface("alpha", "10.0.0.2", 2);

        // Same registration and endpoint; the adapter's name is all that
        // separates them, and it must be enough.
        assert_eq!(wired.registration(), wireless.registration());
        assert_eq!(wired.hostname, wireless.hostname);
        assert_eq!(wired.port, wireless.port);
        assert_ne!(wired.id(), wireless.id());
    }

    #[test]
    fn a_named_occurrence_keeps_its_id_when_its_endpoint_or_addresses_change() {
        let before = on_interface("alpha", "10.0.0.1", 1);

        let mut after = on_interface("alpha", "10.0.0.2", 1);
        after.addresses.push("10.0.0.3".parse().unwrap());
        after.hostname = Some("moved.local".to_string());
        after.port = Some(2222);
        after.txt.insert("path".to_string(), "/admin".to_string());

        // Re-resolving one occurrence updates it in place rather than forking a
        // duplicate, however much of its data moved.
        assert_eq!(before.id(), after.id());
    }

    #[test]
    fn without_an_adapter_name_the_endpoint_still_separates_occurrences() {
        let mut first = Entry::new("alpha", "_ssh._tcp", "local");
        first.hostname = Some("a.local".to_string());
        first.port = Some(22);
        let mut second = Entry::new("alpha", "_ssh._tcp", "local");
        second.hostname = Some("b.local".to_string());
        second.port = Some(22);

        assert_ne!(first.id(), second.id());
        // Addresses are not identity: a service's address set can change
        // without it becoming a different service.
        let mut readdressed = first.clone();
        readdressed.addresses = vec!["10.0.0.9".parse().unwrap()];
        assert_eq!(first.id(), readdressed.id());
    }

    #[test]
    fn occurrences_of_one_registration_group_into_one_logical_service() {
        let groups = browse_groups(
            &[
                on_interface("alpha", "10.0.0.1", 1),
                on_interface("alpha", "10.0.0.2", 2),
            ],
            BrowseMode::LogicalService,
        );

        // Grouping is presentation: it merges the occurrences for display
        // without erasing the identity that keeps them independently removable.
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].label(), "alpha");
        assert_eq!(groups[0].instances().len(), 2);
        assert_ne!(groups[0].instances()[0].id(), groups[0].instances()[1].id());
    }

    #[test]
    fn separator_characters_in_names_cannot_collide_ids() {
        // With a joined-string id, `a|b` + `c` and `a` + `b|c` were equal.
        let first = Entry::new("a|b", "c", "local");
        let second = Entry::new("a", "b|c", "local");

        assert_ne!(first.id(), second.id());
        assert_ne!(first.registration(), second.registration());
    }

    #[test]
    fn separator_characters_in_names_cannot_collide_group_keys() {
        // With a `|`-joined group key, `a|b` + `c` and `a` + `b|c` bucketed
        // into the same group and were shown as one service.
        let first = Entry::new("a|b", "c", "local");
        let second = Entry::new("a", "b|c", "local");

        let groups = browse_groups(&[first, second], BrowseMode::LogicalService);

        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn txt_field_lookup_strips_the_prefix_exactly_once() {
        let mut record = Entry::new("nas", "_http._tcp", "local");
        record
            .txt
            .insert("txt.path".to_string(), "nested".to_string());
        record.txt.insert("path".to_string(), "plain".to_string());

        assert_eq!(record.field("txt.path").as_deref(), Some("plain"));
        // A TXT key literally named `txt.path` is reachable as `txt.txt.path`.
        assert_eq!(record.field("txt.txt.path").as_deref(), Some("nested"));
        assert_eq!(record.field("txt.PATH").as_deref(), Some("plain"));
    }

    #[test]
    fn searchable_text_includes_every_instance_field() {
        let mut record = Entry::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        record.addresses = vec!["192.0.2.5".parse().unwrap()];
        record.port = Some(2222);
        record
            .txt
            .insert("note".to_string(), "third floor".to_string());

        let text = record.searchable_text();
        for needle in [
            "alpha",
            "_ssh._tcp",
            "local",
            "alpha.local",
            "192.0.2.5",
            "2222",
            "note",
            "third floor",
        ] {
            assert!(text.contains(needle), "missing `{needle}` in `{text}`");
        }
    }

    #[test]
    fn grouping_by_host_buckets_records_and_labels_unresolved() {
        let a = resolved("alpha", "_ssh._tcp");
        let b = resolved("beta", "_http._tcp");
        let pending = Entry::new("ghost", "_ipp._tcp", "local");

        let groups = browse_groups(&[a, b, pending], BrowseMode::Host);

        let labels: Vec<&str> = groups.iter().map(|g| g.label()).collect();
        assert!(labels.contains(&"alpha.local"));
        assert!(labels.contains(&"beta.local"));
        assert!(labels.contains(&"<unresolved host>"));
    }

    // ── mode-aware projections ─────────────────────────────────────────────

    /// One service on `host`, with its own type, port, and TXT data.
    fn service_on(
        name: &str,
        service_type: &str,
        host: &str,
        port: u16,
        txt: &[(&str, &str)],
    ) -> Entry {
        let mut record = Entry::new(name, service_type, "local");
        record.hostname = Some(host.to_string());
        record.addresses = vec!["192.0.2.1".parse().unwrap()];
        record.port = Some(port);
        record.txt = txt
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        record
    }

    #[test]
    fn a_host_row_states_only_its_host_and_lists_every_service_on_it() {
        // One host offering SSH and HTTP on different ports with different TXT.
        let shell = service_on("shell", "_ssh._tcp", "nas.local", 22, &[("v", "2")]);
        let site = service_on("site", "_http._tcp", "nas.local", 80, &[("path", "/admin")]);

        let groups = browse_groups(&[shell, site], BrowseMode::Host);

        assert_eq!(groups.len(), 1);
        let host = &groups[0];
        assert_eq!(host.label(), "nas.local");
        // The row vouches for the host and nothing else: a `HostAggregate` has
        // no service type, port, or TXT field to mistake for host-wide data.
        assert_eq!(
            *host.facts(),
            GroupFacts::Host(HostAggregate {
                hostname: Some("nas.local".to_string()),
            })
        );
        assert_eq!(host.facts().host(), RowHost::Resolved("nas.local"));
        assert_eq!(host.facts().service_type(), RowServiceType::Varies);

        // Both services are listed, each keeping its own type and port.
        let children = host.child_services();
        let listed: Vec<(&str, &str, Option<u16>)> = children
            .iter()
            .map(|child| {
                (
                    child.facts.name.as_str(),
                    child.facts.service_type.as_str(),
                    child.facts.port,
                )
            })
            .collect();
        assert_eq!(
            listed,
            vec![
                ("shell", "_ssh._tcp", Some(22)),
                ("site", "_http._tcp", Some(80)),
            ]
        );
        assert_eq!(host.service_types(), vec!["_http._tcp", "_ssh._tcp"]);
        assert_eq!(host.logical_service_count(), 2);
        assert_eq!(host.occurrence_count(), 2);
    }

    #[test]
    fn a_service_type_row_states_only_its_type_and_lists_every_host_offering_it() {
        let alpha = service_on("alpha", "_ssh._tcp", "alpha.local", 22, &[("os", "linux")]);
        let beta = service_on("beta", "_ssh._tcp", "beta.local", 2222, &[("os", "bsd")]);
        let pending = Entry::new("ghost", "_ssh._tcp", "local");

        let groups = browse_groups(&[alpha, beta, pending], BrowseMode::ServiceType);

        assert_eq!(groups.len(), 1);
        let by_type = &groups[0];
        assert_eq!(by_type.label(), "_ssh._tcp");
        assert_eq!(
            by_type.facts().service_type(),
            RowServiceType::Invariant("_ssh._tcp")
        );
        // No host is type-wide, so the row refuses to name one at all.
        assert_eq!(by_type.facts().host(), RowHost::Varies);

        let children = by_type.child_services();
        let hosts: Vec<Option<&str>> = children
            .iter()
            .map(|child| child.facts.hostname.as_deref())
            .collect();
        assert_eq!(hosts, vec![Some("alpha.local"), Some("beta.local"), None]);
        // Each child keeps its own port rather than sharing a row-wide one.
        let ports: Vec<Option<u16>> = children.iter().map(|child| child.facts.port).collect();
        assert_eq!(ports, vec![Some(22), Some(2222), None]);

        // Three logical services and occurrences, but only two resolved hosts:
        // the unresolved one is not a host.
        assert_eq!(by_type.logical_service_count(), 3);
        assert_eq!(by_type.occurrence_count(), 3);
        assert_eq!(by_type.resolved_host_count(), 2);
    }

    #[test]
    fn duplicate_labels_keep_a_deterministic_order_across_input_permutations() {
        // Three rows whose labels are all `shell`: only the structured identity
        // separates them, so only it can order them.
        let on_nas = service_on("shell", "_ssh._tcp", "nas.local", 22, &[]);
        let on_pi = service_on("shell", "_ssh._tcp", "pi.local", 22, &[]);
        let other_port = service_on("shell", "_ssh._tcp", "nas.local", 2222, &[]);

        let forward = browse_groups(
            &[on_nas.clone(), on_pi.clone(), other_port.clone()],
            BrowseMode::LogicalService,
        );
        let reversed = browse_groups(&[other_port, on_pi, on_nas], BrowseMode::LogicalService);

        assert_eq!(forward.len(), 3);
        assert!(forward.iter().all(|group| group.label() == "shell"));
        // Same records, any input order, same row order.
        let ids = |groups: &[EntryGroup]| {
            groups
                .iter()
                .map(|group| group.id().clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&forward), ids(&reversed));

        let endpoints: Vec<(Option<&str>, Option<u16>)> = forward
            .iter()
            .map(|group| match group.facts() {
                GroupFacts::LogicalService(service) => (service.hostname.as_deref(), service.port),
                other => panic!("expected a logical service, got {other:?}"),
            })
            .collect();
        assert_eq!(
            endpoints,
            vec![
                (Some("nas.local"), Some(22)),
                (Some("nas.local"), Some(2222)),
                (Some("pi.local"), Some(22)),
            ]
        );
    }

    #[test]
    fn the_unresolved_host_row_is_an_identity_not_a_label() {
        // A device advertising the sentinel as its hostname has resolved a host
        // and must not be collected into the unresolved row beside it.
        let mut impostor = Entry::new("impostor", "_ssh._tcp", "local");
        impostor.hostname = Some(UNRESOLVED_HOST_LABEL.to_string());
        impostor.port = Some(22);
        let pending = Entry::new("ghost", "_ipp._tcp", "local");

        let groups = browse_groups(&[impostor, pending], BrowseMode::Host);

        assert_eq!(groups.len(), 2);
        // Both rows read identically; only their identities tell them apart.
        assert!(
            groups
                .iter()
                .all(|group| group.label() == UNRESOLVED_HOST_LABEL)
        );
        assert_eq!(
            *groups[0].id(),
            EntryGroupId::Host(HostKey::Resolved(UNRESOLVED_HOST_LABEL.to_string()))
        );
        assert_eq!(
            groups[0].facts().host(),
            RowHost::Resolved(UNRESOLVED_HOST_LABEL)
        );
        // Identical labels, so the structured identity settles the order.
        assert_eq!(*groups[1].id(), EntryGroupId::Host(HostKey::Unresolved));
        assert_eq!(groups[1].facts().host(), RowHost::Unresolved);
    }

    #[test]
    fn logical_service_txt_is_shared_only_where_every_occurrence_agrees() {
        let mut wired = on_interface("alpha", "10.0.0.1", 1);
        wired.txt.insert("model".to_string(), "rpi5".to_string());
        wired.txt.insert("iface".to_string(), "eth0".to_string());
        wired.txt.insert("wired".to_string(), "yes".to_string());
        let mut wireless = on_interface("alpha", "10.0.0.2", 2);
        wireless.txt.insert("model".to_string(), "rpi5".to_string());
        wireless
            .txt
            .insert("iface".to_string(), "wlan0".to_string());

        let groups = browse_groups(&[wired, wireless], BrowseMode::LogicalService);

        assert_eq!(groups.len(), 1);
        let txt = groups[0].txt();
        // Agreed on by every occurrence.
        assert_eq!(
            txt.get("model"),
            Some(&TxtValue::Shared("rpi5".to_string()))
        );
        // Disagreed on: neither value describes the service.
        assert_eq!(txt.get("iface"), Some(&TxtValue::Mixed));
        // Carried by only one occurrence, so not service-wide either.
        assert_eq!(txt.get("wired"), Some(&TxtValue::Mixed));
    }

    #[test]
    fn row_counts_agree_with_the_rows_each_projection_builds() {
        let records = vec![
            service_on("shell", "_ssh._tcp", "nas.local", 22, &[]),
            service_on("site", "_http._tcp", "nas.local", 80, &[]),
            service_on("shell", "_ssh._tcp", "pi.local", 22, &[]),
            Entry::new("ghost", "_ipp._tcp", "local"),
        ];

        for mode in [
            BrowseMode::LogicalService,
            BrowseMode::Host,
            BrowseMode::ServiceType,
        ] {
            assert_eq!(
                browse_row_count(&records, mode),
                browse_groups(&records, mode).len(),
                "{mode:?} count must match the rows it would build"
            );
        }
        assert_eq!(browse_row_count(&records, BrowseMode::LogicalService), 4);
        // Two resolved hosts plus the one unresolved row.
        assert_eq!(browse_row_count(&records, BrowseMode::Host), 3);
        assert_eq!(browse_row_count(&records, BrowseMode::ServiceType), 3);
    }

    #[test]
    fn grouping_by_service_type_merges_same_type() {
        let a = resolved("alpha", "_ssh._tcp");
        let b = resolved("beta", "_ssh._tcp");
        let c = resolved("gamma", "_http._tcp");

        let groups = browse_groups(&[a, b, c], BrowseMode::ServiceType);

        assert_eq!(groups.len(), 2);
        let ssh = groups
            .iter()
            .find(|g| g.label() == "_ssh._tcp")
            .expect("ssh group");
        assert_eq!(ssh.instances().len(), 2);
    }

    #[test]
    fn groups_are_sorted_by_label() {
        let groups = browse_groups(
            &[
                resolved("charlie", "_ssh._tcp"),
                resolved("alpha", "_ssh._tcp"),
                resolved("bravo", "_ssh._tcp"),
            ],
            BrowseMode::LogicalService,
        );

        let labels: Vec<&str> = groups.iter().map(|g| g.label()).collect();
        assert_eq!(labels, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn decode_handles_incomplete_and_out_of_range_escapes() {
        // Fewer than three digits: the backslash and digits are kept verbatim.
        assert_eq!(decode_dns_sd_escapes(r"a\09b"), r"a\09b");
        // A trailing lone backslash is preserved.
        assert_eq!(decode_dns_sd_escapes(r"a\"), r"a\");
        // `\999` is three digits but 999 does not fit in a byte, so it is kept.
        assert_eq!(decode_dns_sd_escapes(r"x\999y"), r"x\999y");
    }

    #[test]
    fn only_the_command_tab_lacks_a_browse_projection() {
        assert_eq!(
            GroupingMode::LogicalService.browse_mode(),
            Some(BrowseMode::LogicalService)
        );
        assert_eq!(GroupingMode::Host.browse_mode(), Some(BrowseMode::Host));
        assert_eq!(
            GroupingMode::ServiceType.browse_mode(),
            Some(BrowseMode::ServiceType)
        );
        // The command tab lists configured rules, not a projection of the
        // discovered entries, so it has no browse mode to ask for.
        assert_eq!(GroupingMode::Command.browse_mode(), None);
    }
}
