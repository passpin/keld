use crate::{FunctionId, IrBlockId, Module};
use keld_native_abi::{AllocationPhase, allocation_site_id, checked_allocation_site_id};
use std::collections::{BTreeMap, BTreeSet};

/// The ordered coordinate used to assign a semantic allocation base ID.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AllocationCoordinate {
    pub function: FunctionId,
    pub block: IrBlockId,
    pub instruction_index: u32,
}

/// Deterministic Native-1 semantic allocation-site table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationSchedule {
    bases: BTreeMap<AllocationCoordinate, u32>,
}

impl AllocationSchedule {
    /// Assigns one-based base IDs by the frozen function/block/index order.
    ///
    /// Every instruction coordinate is retained so test controls can describe
    /// a complete execution schedule, while only semantic allocation phases
    /// are consumed by either engine.
    #[must_use]
    pub fn from_module(module: &Module) -> Self {
        let mut coordinates = BTreeSet::new();
        let main_entry = module
            .functions
            .iter()
            .find(|function| function.id == module.main)
            .map_or(IrBlockId(0), |function| function.entry);
        coordinates.insert(AllocationCoordinate {
            function: module.main,
            block: main_entry,
            instruction_index: 0,
        });
        for function in &module.functions {
            for block in &function.blocks {
                for (instruction_index, _instruction) in block.instructions.iter().enumerate() {
                    coordinates.insert(AllocationCoordinate {
                        function: function.id,
                        block: block.id,
                        instruction_index: u32::try_from(instruction_index).unwrap_or(u32::MAX),
                    });
                }
            }
        }
        let bases = coordinates
            .into_iter()
            .enumerate()
            .map(|(index, coordinate)| (coordinate, u32::try_from(index + 1).unwrap_or(u32::MAX)))
            .collect();
        Self { bases }
    }

    /// Returns the frozen base ID for one executable instruction coordinate.
    #[must_use]
    pub fn base_id(
        &self,
        function: FunctionId,
        block: IrBlockId,
        instruction_index: u32,
    ) -> Option<u32> {
        self.bases
            .get(&AllocationCoordinate {
                function,
                block,
                instruction_index,
            })
            .copied()
    }

    /// Returns all ordered coordinates and base IDs for audit tooling.
    pub fn coordinates(&self) -> impl Iterator<Item = (AllocationCoordinate, u32)> + '_ {
        self.bases
            .iter()
            .map(|(coordinate, base)| (*coordinate, *base))
    }

    /// Computes a phase and structural-copy ordinal site ID from a base ID.
    #[must_use]
    pub fn site_id(&self, base_id: u32, phase: AllocationPhase, ordinal: u32) -> u32 {
        allocation_site_id(base_id, phase, ordinal)
    }

    /// Checked variant used by validation/audit code before emitting IDs.
    #[must_use]
    pub fn checked_site_id(
        &self,
        base_id: u32,
        phase: AllocationPhase,
        ordinal: u32,
    ) -> Option<u32> {
        checked_allocation_site_id(base_id, phase, ordinal)
    }
}
