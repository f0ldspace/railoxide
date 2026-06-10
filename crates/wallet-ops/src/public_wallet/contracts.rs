use alloy::sol;

sol! {
    interface PublicErc20 {
        function balanceOf(address account) external view returns (uint256);
        function transfer(address recipient, uint256 amount) external returns (bool);
    }

    interface Multicall3Balance {
        function getEthBalance(address addr) external view returns (uint256);
    }
}
