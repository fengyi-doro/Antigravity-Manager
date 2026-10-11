import { Link, useLocation } from 'react-router-dom';
import { NavigationDropdown } from './NavDropdowns';
import { isActive, getCurrentNavItem, type NavItem } from './constants';
import { useConfigStore } from '../../stores/useConfigStore';

interface NavMenuProps {
    navItems: NavItem[];
}

/** Navigation collapses before the menu can crowd the logo and account controls. */
export function NavMenu({ navItems }: NavMenuProps) {
    const location = useLocation();
    const { isMenuItemHidden } = useConfigStore();

    // 过滤隐藏的菜单项
    const visibleNavItems = navItems.filter(item => !isMenuItemHidden(item.path));

    return (
        <>
            {/* 文字胶囊 (≥ 1280px) */}
            <nav className="max-[1279px]:hidden flex items-center gap-1 bg-gray-100 dark:bg-base-200 rounded-full p-1">
                {visibleNavItems.map((item) => (
                    <Link
                        key={item.path}
                        to={item.path}
                        draggable="false"
                        className={`
                            px-2
                            py-2 
                            rounded-full 
                            text-sm 
                            font-medium 
                            transition-all 
                            whitespace-nowrap
                            ${isActive(location.pathname, item.path)
                                ? 'bg-gray-900 text-white shadow-sm dark:bg-white dark:text-gray-900'
                                : 'text-gray-700 hover:text-gray-900 hover:bg-gray-200 dark:text-gray-400 dark:hover:text-base-content dark:hover:bg-base-100'
                            }
                        `}
                    >
                        {item.label}
                    </Link>
                ))}
            </nav>

            {/* 图标胶囊 (880px - 1280px) */}
            <nav className="max-[879px]:hidden min-[1280px]:hidden flex items-center gap-1 bg-gray-100 dark:bg-base-200 rounded-full p-1">
                {visibleNavItems.map((item) => (
                    <Link
                        key={item.path}
                        to={item.path}
                        draggable="false"
                        className={`
                            p-2
                            rounded-full
                            transition-all
                            ${isActive(location.pathname, item.path)
                                ? 'bg-gray-900 text-white shadow-sm dark:bg-white dark:text-gray-900'
                                : 'text-gray-700 hover:text-gray-900 hover:bg-gray-200 dark:text-gray-400 dark:hover:text-base-content dark:hover:bg-base-100'
                            }
                        `}
                        title={item.label}
                    >
                        <item.icon className="w-5 h-5" />
                    </Link>
                ))}
            </nav>

            {/* 图标+文字下拉 (375px - 880px) */}
            <div className="max-[374px]:hidden min-[880px]:hidden block">
                <NavigationDropdown
                    navItems={visibleNavItems}
                    isActive={(path) => isActive(location.pathname, path)}
                    getCurrentNavItem={() => getCurrentNavItem(location.pathname, visibleNavItems)}
                    onNavigate={() => { }}
                    showLabel={true}
                />
            </div>

            {/* 图标下拉 (< 375px) */}
            <div className="min-[375px]:hidden">
                <NavigationDropdown
                    navItems={visibleNavItems}
                    isActive={(path) => isActive(location.pathname, path)}
                    getCurrentNavItem={() => getCurrentNavItem(location.pathname, visibleNavItems)}
                    onNavigate={() => { }}
                    showLabel={false}
                />
            </div>
        </>
    );
}
