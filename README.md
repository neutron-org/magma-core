# Disclaimer
Magma vaults have been deprecated. They are no longer being actively maintained. Magma can no longer safely guarantee the safety of any deposited funds. PLEASE WITHDRAW ALL FUNDS IMMEDIATELY!

## Manual Withdrawl

Funds can be withdrawn manually using the Osmosis CLI[https://docs.osmosis.zone/build/developer-environment/cli/]

1. Query contract balance
   `osmosisd q wasm contract-state smart [MAGMA CONTRACT ADDRESS] '{"balance": {"address": "[YOUR ADDRESS]"}}'`
2. Execute Withdrawal 
  `osmosisd tx wasm execute [MAGMA CONTRACT ADDRESS] '{"withdraw": {"amount0_min": "0", "amount1_min": "0", "shares": "[SHARES]", "to": "[YOUR ADDRESS]"}}' --from [YOUR ADDRESS] --chain-id osmosis-1 --fees 130000uosmo --gas auto --gas-adjustment 1.5 -y`



## License

The code in this repository is licensed under the BSL.  You are free to study and contribute to the code, but not free to independently deploy this code or derivatives thereof.
